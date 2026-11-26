//! Web pages exported from SuperSplat (or splat-transform) with the scene inside: the scene
//! is a base64 `data:` link in the page, usually a bundled SOG, in older exports a compressed
//! PLY. Only the embedded scene is read; nothing in the page is run.

use super::{Limits, Scene};
use std::io::{BufReader, Read};
use std::path::Path;

const MARK: &[u8] = b"data:application/octet-stream;base64,";
/// The scene inside a page read at most this big (encoded).
const MAX_ENCODED: u64 = 2 << 30;
/// Pages smaller than this can't hold a scene worth showing.
const MIN_PAGE: u64 = 16 << 10;

/// Where the embedded scene starts, if the page has one.
fn find(path: &Path) -> Option<u64> {
    let f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() < MIN_PAGE {
        return None;
    }
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut buf = vec![0u8; 1 << 20];
    let mut carry: Vec<u8> = Vec::new();
    let mut at = 0u64;
    loop {
        let n = r.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        let mut hay = std::mem::take(&mut carry);
        let base = at - hay.len() as u64;
        hay.extend_from_slice(&buf[..n]);
        at += n as u64;
        if let Some(p) = hay.windows(MARK.len()).position(|w| w == MARK) {
            return Some(base + (p + MARK.len()) as u64);
        }
        carry = hay[hay.len().saturating_sub(MARK.len())..].to_vec();
    }
}

/// Whether a page carries a splat scene (a quick look for the embedded data).
pub fn has_scene(path: &Path) -> bool {
    find(path).is_some()
}

pub fn read(path: &Path, limits: Limits) -> Result<Scene, String> {
    let start = find(path).ok_or("This page has no splat scene inside.")?;
    let mut f = std::fs::File::open(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    use std::io::Seek;
    f.seek(std::io::SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut text = Vec::new();
    let mut r = BufReader::with_capacity(1 << 20, f).take(MAX_ENCODED);
    let mut buf = vec![0u8; 1 << 20];
    'read: loop {
        let n = r.read(&mut buf).map_err(|e| format!("Couldn't read it: {e}"))?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=' {
                text.push(b);
            } else {
                break 'read;
            }
        }
    }
    while text.len() % 4 != 0 {
        text.push(b'=');
    }
    let data = data_encoding::BASE64.decode(&text).map_err(|_| "The scene in this page is damaged.".to_string())?;
    drop(text);
    let len = data.len() as u64;
    let mut scene = if data.starts_with(b"PK\x03\x04") {
        super::sog::read_zip(std::io::Cursor::new(data), limits)?
    } else if data.starts_with(b"ply") {
        super::ply::read_from(&mut std::io::Cursor::new(data), len, limits)?
    } else if data.starts_with(&[0x1f, 0x8b]) || data.starts_with(b"NGSP") {
        super::spz::parse(&data, limits)?
    } else {
        return Err("The scene in this page is in a format Filescape doesn't know.".into());
    };
    scene.format = format!("{}, inside a web page", scene.format);
    Ok(scene)
}
