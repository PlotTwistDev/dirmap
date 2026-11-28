//! Windows shortcuts (`.lnk`), read as data: where they point, with what arguments, where they
//! start and which icon they show. The format is Microsoft's [MS-SHLLINK]; only the parts
//! Explorer shows under a shortcut's Properties are read.

/// What a shortcut says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shortcut {
    /// The full path it opens, as stored (a Windows path, maybe with %VARIABLES%).
    pub target: Option<String>,
    pub arguments: Option<String>,
    /// "Start in".
    pub working_dir: Option<String>,
    /// "Comment".
    pub comment: Option<String>,
    /// The file its icon comes from, and which icon in it.
    pub icon: Option<(String, i32)>,
    /// The target path relative to the shortcut, used when the full path is missing.
    pub relative: Option<String>,
}

const HAS_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_RELATIVE_PATH: u32 = 0x8;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// A NUL-terminated 8-bit string (the system's code page; read as Latin-1, which is exact for
/// plain ASCII paths).
fn ansi_z(d: &[u8], o: usize) -> Option<String> {
    let s = d.get(o..)?;
    let end = s.iter().position(|&b| b == 0)?;
    let t: String = s[..end].iter().map(|&b| b as char).collect();
    (!t.is_empty()).then_some(t)
}

fn utf16_z(d: &[u8], o: usize) -> Option<String> {
    let s = d.get(o..)?;
    let w: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let t = String::from_utf16_lossy(&w);
    (!t.is_empty()).then_some(t)
}

/// Read a shortcut. None when the data isn't one.
pub fn shortcut(d: &[u8]) -> Option<Shortcut> {
    const CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];
    if u32_at(d, 0)? != 0x4c || d.get(4..20)? != CLSID {
        return None;
    }
    let flags = u32_at(d, 0x14)?;
    let icon_index = u32_at(d, 0x38)? as i32;
    let mut s = Shortcut::default();
    let mut o = 0x4c;
    if flags & HAS_ID_LIST != 0 {
        o += 2 + u16_at(d, o)? as usize;
    }
    if flags & HAS_LINK_INFO != 0 {
        let size = u32_at(d, o)? as usize;
        s.target = link_info(d.get(o..o + size)?);
        o += size;
    }
    let unicode = flags & IS_UNICODE != 0;
    let text = |o: &mut usize| -> Option<String> {
        let n = u16_at(d, *o)? as usize;
        *o += 2;
        let t = if unicode {
            let w: Vec<u16> = d.get(*o..*o + n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            *o += n * 2;
            String::from_utf16_lossy(&w)
        } else {
            let t = d.get(*o..*o + n)?.iter().map(|&b| b as char).collect();
            *o += n;
            t
        };
        Some(t)
    };
    let nonempty = |t: Option<String>| t.filter(|t| !t.trim().is_empty());
    if flags & HAS_NAME != 0 {
        s.comment = nonempty(text(&mut o));
    }
    if flags & HAS_RELATIVE_PATH != 0 {
        s.relative = nonempty(text(&mut o));
    }
    if flags & HAS_WORKING_DIR != 0 {
        s.working_dir = nonempty(text(&mut o));
    }
    if flags & HAS_ARGUMENTS != 0 {
        s.arguments = nonempty(text(&mut o));
    }
    if flags & HAS_ICON_LOCATION != 0 {
        s.icon = nonempty(text(&mut o)).map(|p| (p, icon_index));
    }
    // Extra data: a target written with %VARIABLES% (shortcuts installers make to Program
    // Files) when there's no full path above.
    while s.target.is_none() {
        let size = u32_at(d, o)? as usize;
        if size < 8 {
            break;
        }
        if u32_at(d, o + 4)? == 0xa000_0001 {
            s.target = utf16_z(d, o + 8 + 260).or_else(|| ansi_z(d, o + 8));
        }
        o += size;
    }
    if s.target.is_none() && s.relative.is_none() && s.icon.is_none() {
        return None;
    }
    Some(s)
}

/// The full target from a LinkInfo structure: a local path, or a network share and the rest.
fn link_info(li: &[u8]) -> Option<String> {
    let header = u32_at(li, 4)? as usize;
    let flags = u32_at(li, 8)?;
    if flags & 1 != 0 {
        let unicode = (header >= 0x24).then(|| u32_at(li, 28)).flatten().filter(|&o| o != 0).and_then(|o| utf16_z(li, o as usize));
        if let Some(p) = unicode.or_else(|| ansi_z(li, u32_at(li, 16)? as usize)) {
            let suffix = (header >= 0x24).then(|| u32_at(li, 32)).flatten().filter(|&o| o != 0).and_then(|o| utf16_z(li, o as usize)).or_else(|| ansi_z(li, u32_at(li, 24)? as usize));
            return Some(join(&p, suffix.as_deref()));
        }
    }
    if flags & 2 != 0 {
        let net = u32_at(li, 20)? as usize;
        let share = ansi_z(li, net + u32_at(li, net + 8)? as usize)?;
        let suffix = ansi_z(li, u32_at(li, 24)? as usize);
        return Some(join(&share, suffix.as_deref()));
    }
    None
}

fn join(base: &str, suffix: Option<&str>) -> String {
    match suffix.filter(|s| !s.is_empty()) {
        Some(s) if base.ends_with('\\') => format!("{base}{s}"),
        Some(s) => format!("{base}\\{s}"),
        None => base.to_string(),
    }
}

/// A Windows path with its %VARIABLES% filled in from this computer's environment (on Windows;
/// elsewhere they stay as written).
pub fn expand(path: &str) -> String {
    if !cfg!(windows) {
        return path.to_string();
    }
    let mut out = String::new();
    let mut rest = path;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let name = &after[..b];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn counted(s: &str) -> Vec<u8> {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut v = (w.len() as u16).to_le_bytes().to_vec();
        v.extend(w.iter().flat_map(|c| c.to_le_bytes()));
        v
    }

    /// A shortcut to `target` (a local path, split as Windows stores it), with arguments,
    /// start-in folder and icon.
    pub(crate) fn sample_lnk(target: &str) -> Vec<u8> {
        let mut d = vec![0u8; 0x4c];
        d[0..4].copy_from_slice(&0x4cu32.to_le_bytes());
        d[4..20].copy_from_slice(&[0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
        let flags = HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ARGUMENTS | HAS_ICON_LOCATION | IS_UNICODE;
        d[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        d[0x38..0x3c].copy_from_slice(&2u32.to_le_bytes());
        // LinkInfo with a 0x1c header: volume (empty), local base path, empty suffix.
        let base = format!("{target}\0");
        let mut li = vec![0u8; 0x1c];
        let vol_at = 0x1c;
        let vol = [0x10u8, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0];
        let base_at = vol_at + vol.len();
        let suffix_at = base_at + base.len();
        li.extend_from_slice(&vol);
        li.extend_from_slice(base.as_bytes());
        li.push(0);
        let size = li.len() as u32;
        li[0..4].copy_from_slice(&size.to_le_bytes());
        li[4..8].copy_from_slice(&0x1cu32.to_le_bytes());
        li[8..12].copy_from_slice(&1u32.to_le_bytes());
        li[12..16].copy_from_slice(&(vol_at as u32).to_le_bytes());
        li[16..20].copy_from_slice(&(base_at as u32).to_le_bytes());
        li[24..28].copy_from_slice(&(suffix_at as u32).to_le_bytes());
        d.extend(li);
        d.extend(counted("Paint pictures"));
        d.extend(counted("C:\\Users\\Rich\\Pictures"));
        d.extend(counted("--new-window"));
        d.extend(counted("%ProgramFiles%\\Acme\\paint.exe"));
        d.extend_from_slice(&[0; 4]);
        d
    }

    #[test]
    fn reads_a_shortcut() {
        let s = shortcut(&sample_lnk("C:\\Program Files\\Acme\\paint.exe")).unwrap();
        assert_eq!(s.target.as_deref(), Some("C:\\Program Files\\Acme\\paint.exe"));
        assert_eq!(s.arguments.as_deref(), Some("--new-window"));
        assert_eq!(s.working_dir.as_deref(), Some("C:\\Users\\Rich\\Pictures"));
        assert_eq!(s.comment.as_deref(), Some("Paint pictures"));
        assert_eq!(s.icon, Some(("%ProgramFiles%\\Acme\\paint.exe".to_string(), 2)));
        assert!(shortcut(b"not a shortcut at all, just some text in a file").is_none());
        let full = sample_lnk("C:\\x.exe");
        for cut in [10, 0x4c, 0x60, full.len() - 3] {
            let _ = shortcut(&full[..cut]);
        }
    }

    #[test]
    fn expands_variables_on_windows_only() {
        let p = "%NO_SUCH_VARIABLE_HERE%\\a.exe";
        assert_eq!(expand(p), p);
        assert_eq!(expand("50% off"), "50% off");
    }
}
