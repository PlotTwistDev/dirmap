//! "Get a small model" for Ask AI: downloads one recommended model, after the user approves it,
//! from its publisher's own Hugging Face repository at a fixed revision, and checks its SHA-256
//! before it's used. This is the only time Ask AI goes online. A stopped download is kept and
//! resumed next time.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// The model offered, with everything shown on the approval screen.
pub struct Starter {
    pub name: &'static str,
    pub publisher: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// Where the user can read about it.
    pub page: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub licence: &'static str,
    pub licence_url: &'static str,
    /// Memory to run it at 4K tokens of context, roughly.
    pub memory: u64,
    pub about: &'static str,
}

/// Qwen 3 1.7B from Qwen's own repository, at commit 90862c4 (8-bit, the only GGUF they publish
/// for this size). Size and checksum are Hugging Face's LFS record of the file.
pub const STARTER: Starter = Starter {
    name: "Qwen3 1.7B",
    publisher: "Qwen (Alibaba Cloud)",
    file: "Qwen3-1.7B-Q8_0.gguf",
    url: "https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/resolve/90862c4b9d2787eaed51d12237eafdfe7c5f6077/Qwen3-1.7B-Q8_0.gguf",
    page: "https://huggingface.co/Qwen/Qwen3-1.7B-GGUF",
    size: 1_834_426_016,
    sha256: "061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a",
    licence: "Apache 2.0",
    licence_url: "https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/blob/main/LICENSE",
    memory: 2_400_000_000,
    about: "A small general-purpose model that answers in many languages and can think before it answers. \
            Good for short questions about files and folders; it knows less than larger models.",
};

/// Where Filescape keeps models it downloads: beside its other data, but not in a roaming
/// profile on Windows.
pub fn folder() -> Option<PathBuf> {
    let home = crate::fsys::places::home();
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join(crate::APP_ID).join("Models"))
    } else if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support").join(crate::APP_ID).join("Models"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")))
            .map(|p| p.join(crate::APP_ID.to_lowercase()).join("models"))
    }
}

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub cancel: AtomicBool,
    /// "Downloading", "Checking", …
    pub stage: Mutex<&'static str>,
}

/// Downloads `s` into `dir`, resuming a part left by an earlier try, and checks it. Returns the
/// finished file. On cancel the part is kept for next time.
pub fn download(s: &Starter, dir: &Path, p: &Progress) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    let dest = dir.join(s.file);
    let part = dir.join(format!("{}.part", s.file));
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > s.size {
        std::fs::remove_file(&part).ok();
        have = 0;
    }
    if let Some(free) = crate::drives::free_at(dir)
        && free < s.size - have + (256 << 20)
    {
        return Err(format!(
            "There isn't enough free space: the model needs {} and the drive has {} free.",
            crate::scan::human(s.size - have),
            crate::scan::human(free)
        ));
    }
    // Hash what an earlier try saved, then carry on from there.
    let mut hasher = Sha256::new();
    if have > 0 {
        *p.stage.lock().unwrap() = "Checking what was already downloaded";
        let mut f = std::fs::File::open(&part).map_err(|e| e.to_string())?;
        hash_into(&mut f, &mut hasher, have, p)?;
    }
    p.done.store(have, Ordering::Relaxed);
    *p.stage.lock().unwrap() = "Downloading";
    if have < s.size {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .tls_config(tls)
            .build()
            .new_agent();
        let mut req = agent.get(s.url).header("User-Agent", "Filescape");
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let mut resp = req.call().map_err(|e| format!("The download couldn't start: {e}"))?;
        let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&part).map_err(|e| e.to_string())?;
        if have > 0 && resp.status().as_u16() != 206 {
            // The server sent the whole file: start again.
            out.set_len(0).map_err(|e| e.to_string())?;
            hasher = Sha256::new();
            have = 0;
            p.done.store(0, Ordering::Relaxed);
        }
        let mut body = resp.body_mut().with_config().limit(s.size - have).reader();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if p.cancel.load(Ordering::Relaxed) {
                return Err("Stopped. What was downloaded is kept, so it carries on from there next time.".into());
            }
            let n = body.read(&mut buf).map_err(|e| format!("The download stopped: {e}. Try again to carry on from where it stopped."))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| format!("Can't save the download: {e}"))?;
            hasher.update(&buf[..n]);
            have += n as u64;
            p.done.store(have, Ordering::Relaxed);
        }
        out.sync_all().ok();
    }
    *p.stage.lock().unwrap() = "Checking";
    if have != s.size {
        return Err("The download ended early. Try again to carry on from where it stopped.".into());
    }
    let got = hex(&hasher.finalize());
    if got != s.sha256 {
        std::fs::remove_file(&part).ok();
        return Err("The downloaded file isn't the one expected (its checksum doesn't match), so it was removed. Try again.".into());
    }
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

fn hash_into(f: &mut impl Read, hasher: &mut Sha256, len: u64, p: &Progress) -> Result<(), String> {
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    while done < len {
        if p.cancel.load(Ordering::Relaxed) {
            return Err("Stopped.".into());
        }
        let want = buf.len().min((len - done) as usize);
        let n = f.read(&mut buf[..want]).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        p.done.store(done, Ordering::Relaxed);
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_is_pinned_to_a_revision_and_checksum() {
        assert!(STARTER.url.starts_with("https://huggingface.co/Qwen/"));
        assert!(STARTER.url.contains("/resolve/90862c4b9d2787eaed51d12237eafdfe7c5f6077/"));
        assert!(STARTER.url.ends_with(STARTER.file));
        assert_eq!(STARTER.sha256.len(), 64);
        assert!(STARTER.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    /// Serves `body` over plain HTTP, honouring `Range: bytes=N-`, for `n` requests.
    fn serve(body: &'static [u8], n: usize) -> String {
        use std::io::BufRead;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/m.gguf", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for s in l.incoming().take(n) {
                let mut s = s.unwrap();
                let mut r = std::io::BufReader::new(s.try_clone().unwrap());
                let mut from = 0usize;
                loop {
                    let mut line = String::new();
                    r.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                }
                let head = if from > 0 {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nConnection: close\r\n\r\n", body.len() - from, body.len() - 1, body.len())
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                };
                s.write_all(head.as_bytes()).unwrap();
                s.write_all(&body[from..]).unwrap();
            }
        });
        url
    }

    fn starter(url: String, body: &[u8], sha: Option<&str>) -> Starter {
        Starter {
            file: "m.gguf",
            url: Box::leak(url.into_boxed_str()),
            size: body.len() as u64,
            sha256: Box::leak(sha.map(str::to_string).unwrap_or_else(|| hex(&Sha256::digest(body))).into_boxed_str()),
            ..STARTER
        }
    }

    #[test]
    fn downloads_resumes_and_checks() {
        let body: &'static [u8] = Vec::leak((0..3_000_000usize).map(|i| (i * 7 % 251) as u8).collect());
        let dir = std::env::temp_dir().join(format!("filescape-getmodel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A whole download.
        let s = starter(serve(body, 1), body, None);
        let got = download(&s, &dir, &Progress::default()).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), body);
        assert!(!dir.join("m.gguf.part").exists());
        // A resumed one: an earlier try left the first million bytes.
        std::fs::remove_file(&got).unwrap();
        std::fs::write(dir.join("m.gguf.part"), &body[..1_000_000]).unwrap();
        let s = starter(serve(body, 1), body, None);
        let got = download(&s, &dir, &Progress::default()).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), body);
        // A file that isn't the one expected is removed, not kept.
        std::fs::remove_file(&got).unwrap();
        let s = starter(serve(body, 1), body, Some(&"0".repeat(64)));
        let e = download(&s, &dir, &Progress::default()).unwrap_err();
        assert!(e.contains("checksum"), "{e}");
        assert!(!dir.join("m.gguf").exists() && !dir.join("m.gguf.part").exists());
        // Stopping keeps nothing broken: no finished file appears.
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        let s = starter(serve(body, 1), body, None);
        assert!(download(&s, &dir, &p).is_err());
        assert!(!dir.join("m.gguf").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hex_is_lower_case() {
        assert_eq!(hex(&[0x0a, 0xff]), "0aff");
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
