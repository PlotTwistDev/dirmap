//! Windows programs (`.exe`), read as data: the version information Explorer shows under
//! Properties > Details (product, company, version), what the program is built for, and its own
//! icon. Nothing in the file is run. Works on every system, so a Windows drive looked at from a
//! Mac shows the same.

/// What a Windows program's header and version information say about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    pub product: Option<String>,
    pub description: Option<String>,
    pub company: Option<String>,
    pub product_version: Option<String>,
    pub file_version: Option<String>,
    pub copyright: Option<String>,
    pub original_name: Option<String>,
    /// "64-bit (x64)", "32-bit (x86)", "ARM64".
    pub arch: Option<String>,
    /// A console (command-line) program rather than one with windows.
    pub console: bool,
    /// The file carries a digital signature (Authenticode). Whether Windows trusts it is only
    /// checked when it's run.
    pub signed: bool,
}

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_VERSION: u32 = 16;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// The parts of a PE file this needs: where the sections are, and the resource directory.
struct Pe<'a> {
    /// The file from its start (all of it, or just the headers).
    data: &'a [u8],
    /// The resource section, read on its own from a big file: (file offset, bytes).
    rsrc: Option<(usize, &'a [u8])>,
    machine: u16,
    subsystem: u16,
    /// (virtual address, virtual size, file offset, size in the file) per section.
    sections: Vec<(u32, u32, u32, u32)>,
    /// The resource directory's address and size.
    resources: Option<(u32, u32)>,
    signed: bool,
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Option<Pe<'a>> {
        if data.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(data, 0x3c)? as usize;
        if data.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let machine = u16_at(data, coff)?;
        let nsec = u16_at(data, coff + 2)? as usize;
        let opt_size = u16_at(data, coff + 16)? as usize;
        let opt = coff + 20;
        let (dirs, count_at) = match u16_at(data, opt)? {
            0x10b => (opt + 96, opt + 92),
            0x20b => (opt + 112, opt + 108),
            _ => return None,
        };
        let subsystem = u16_at(data, opt + 68)?;
        let ndirs = u32_at(data, count_at)? as usize;
        let dir = |i: usize| (i < ndirs).then(|| Some((u32_at(data, dirs + i * 8)?, u32_at(data, dirs + i * 8 + 4)?))).flatten().filter(|d| d.0 != 0 && d.1 != 0);
        let resources = dir(2);
        // The security directory holds the Authenticode signature (its address is a file offset).
        let signed = dir(4).is_some();
        let mut sections = Vec::with_capacity(nsec.min(96));
        let table = opt + opt_size;
        for i in 0..nsec.min(96) {
            let s = table + i * 40;
            sections.push((u32_at(data, s + 12)?, u32_at(data, s + 8)?, u32_at(data, s + 20)?, u32_at(data, s + 16)?));
        }
        Some(Pe { data, rsrc: None, machine, subsystem, sections, resources, signed })
    }

    /// `len` bytes at file offset `o`, from the resource section read apart or the rest.
    fn at(&self, o: usize, len: usize) -> Option<&'a [u8]> {
        if let Some((start, r)) = self.rsrc
            && o >= start
            && o - start < r.len()
        {
            return r.get(o - start..o - start + len);
        }
        self.data.get(o..o.checked_add(len)?)
    }

    fn u16(&self, o: usize) -> Option<u16> {
        u16_at(self.at(o, 2)?, 0)
    }

    fn u32(&self, o: usize) -> Option<u32> {
        u32_at(self.at(o, 4)?, 0)
    }

    /// Where the resource section is in the file: (offset, length).
    fn rsrc_range(&self) -> Option<(usize, usize)> {
        let (rva, _) = self.resources?;
        self.sections.iter().find(|&&(va, vsize, _, rsize)| rva >= va && rva < va + vsize.max(rsize)).map(|&(_, _, raw, rsize)| (raw as usize, rsize as usize))
    }

    /// The file offset of a virtual address.
    fn offset(&self, rva: u32) -> Option<usize> {
        self.sections.iter().find_map(|&(va, vsize, raw, rsize)| {
            let span = vsize.max(rsize);
            (rva >= va && rva < va.checked_add(span)?).then(|| (raw + (rva - va)) as usize)
        })
    }

    fn bytes(&self, rva: u32, len: u32) -> Option<&'a [u8]> {
        self.at(self.offset(rva)?, len as usize)
    }

    /// The entries of the resource directory at `dir` (relative to the resource section):
    /// (id, or u32::MAX for a named entry; is a directory; offset).
    fn entries(&self, root: usize, dir: usize) -> Vec<(u32, bool, usize)> {
        let at = root + dir;
        let (Some(named), Some(ids)) = (self.u16(at + 12), self.u16(at + 14)) else { return Vec::new() };
        (0..(named as usize + ids as usize).min(4096))
            .filter_map(|i| {
                let e = at + 16 + i * 8;
                let name = self.u32(e)?;
                let off = self.u32(e + 4)?;
                let id = if name & 0x8000_0000 != 0 { u32::MAX } else { name };
                Some((id, off & 0x8000_0000 != 0, (off & 0x7fff_ffff) as usize))
            })
            .collect()
    }

    /// The data of resource `kind`: the one with this id (any if None), first language.
    fn resource(&self, kind: u32, id: Option<u32>) -> Option<&'a [u8]> {
        let (rva, _) = self.resources?;
        let root = self.offset(rva)?;
        let (_, _, types) = self.entries(root, 0).into_iter().find(|e| e.0 == kind && e.1)?;
        let (_, is_dir, names) = self.entries(root, types).into_iter().find(|e| id.is_none_or(|id| e.0 == id))?;
        let leaf = if is_dir { self.entries(root, names).into_iter().find(|e| !e.1)?.2 } else { names };
        let (drva, dsize) = (self.u32(root + leaf)?, self.u32(root + leaf + 4)?);
        self.bytes(drva, dsize)
    }
}

/// The most of a program's resource section read for its icon and details.
const RSRC_MAX: usize = 16 << 20;

/// Read the headers and resource section of a program file, without reading the code between
/// (programs can be hundreds of megabytes): (start of the file, the resource section).
fn load(path: &std::path::Path) -> Option<(Vec<u8>, Option<(usize, Vec<u8>)>)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 8192];
    let n = f.read(&mut head).ok()?;
    head.truncate(n);
    let (start, len) = Pe::parse(&head)?.rsrc_range().filter(|r| r.1 <= RSRC_MAX)?;
    if start + len <= head.len() {
        return Some((head, None));
    }
    f.seek(SeekFrom::Start(start as u64)).ok()?;
    let mut r = Vec::with_capacity(len);
    f.take(len as u64).read_to_end(&mut r).ok()?;
    Some((head, Some((start, r))))
}

/// [`program`] for a file on disk.
pub fn program_file(path: &std::path::Path) -> Option<Program> {
    let (head, rsrc) = load(path).or_else(|| {
        // No resources: the header alone still says what it's built for.
        use std::io::Read;
        let mut head = vec![0u8; 8192];
        let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
        head.truncate(n);
        Some((head, None))
    })?;
    let mut pe = Pe::parse(&head)?;
    pe.rsrc = rsrc.as_ref().map(|(o, r)| (*o, &r[..]));
    Some(read_program(&pe))
}

/// [`icon`] for a file on disk.
pub fn icon_file(path: &std::path::Path, size: u32) -> Option<image::RgbaImage> {
    let (head, rsrc) = load(path)?;
    let mut pe = Pe::parse(&head)?;
    pe.rsrc = rsrc.as_ref().map(|(o, r)| (*o, &r[..]));
    icon_of(&pe, size)
}

/// Read a Windows program's details. None when the data isn't a Windows program.
#[cfg(test)]
pub fn program(data: &[u8]) -> Option<Program> {
    Some(read_program(&Pe::parse(data)?))
}

fn read_program(pe: &Pe) -> Program {
    let mut p = Program {
        arch: match pe.machine {
            0x8664 => Some("64-bit (x64)".into()),
            0x14c => Some("32-bit (x86)".into()),
            0xaa64 => Some("ARM64".into()),
            0x1c0 | 0x1c4 => Some("ARM (32-bit)".into()),
            _ => None,
        },
        console: pe.subsystem == 3,
        signed: pe.signed,
        ..Default::default()
    };
    if let Some(v) = pe.resource(RT_VERSION, None) {
        version_strings(v, &mut p);
    }
    p
}

/// One block of a version resource: (key, value bytes, value is text, children bytes).
fn block(d: &[u8]) -> Option<(String, &[u8], bool, &[u8], usize)> {
    let len = (u16_at(d, 0)? as usize).min(d.len());
    let vlen = u16_at(d, 2)? as usize;
    let text = u16_at(d, 4)? == 1;
    let mut key = Vec::new();
    let mut o = 6;
    while let Some(c) = u16_at(d, o) {
        o += 2;
        if c == 0 {
            break;
        }
        key.push(c);
    }
    let o = (o + 3) & !3;
    let vbytes = if text { vlen * 2 } else { vlen };
    let value = d.get(o..(o + vbytes).min(len)).unwrap_or(&[]);
    let kids = (o + vbytes + 3) & !3;
    let children = d.get(kids.min(len)..len).unwrap_or(&[]);
    Some((String::from_utf16_lossy(&key), value, text, children, len.max(6)))
}

/// Walk the blocks in `d` one after another.
fn blocks(mut d: &[u8]) -> Vec<(String, &[u8], bool, &[u8])> {
    let mut out = Vec::new();
    while d.len() >= 6 {
        let Some((k, v, t, c, len)) = block(d) else { break };
        out.push((k, v, t, c));
        let next = (len + 3) & !3;
        if next >= d.len() || out.len() > 512 {
            break;
        }
        d = &d[next..];
    }
    out
}

fn utf16(v: &[u8]) -> Option<String> {
    let w: Vec<u16> = v.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let s = String::from_utf16_lossy(&w).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The strings of VS_VERSIONINFO: the first table, preferring US English.
fn version_strings(v: &[u8], p: &mut Program) {
    let Some((key, fixed, _, children, _)) = block(v) else { return };
    if key != "VS_VERSION_INFO" {
        return;
    }
    // The numeric versions, used when the strings leave them out.
    let number = |ms: u32, ls: u32| format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff);
    if u32_at(fixed, 0) == Some(0xfeef_04bd) {
        p.file_version = Some(number(u32_at(fixed, 8).unwrap_or(0), u32_at(fixed, 12).unwrap_or(0)));
        p.product_version = Some(number(u32_at(fixed, 16).unwrap_or(0), u32_at(fixed, 20).unwrap_or(0)));
    }
    for (k, _, _, kids) in blocks(children) {
        if k != "StringFileInfo" {
            continue;
        }
        let tables = blocks(kids);
        let table = tables.iter().find(|t| t.0.eq_ignore_ascii_case("040904b0") || t.0.to_ascii_lowercase().starts_with("0409")).or(tables.first());
        let Some((_, _, _, strings)) = table else { continue };
        for (name, value, _, _) in blocks(strings) {
            let Some(s) = utf16(value) else { continue };
            match name.as_str() {
                "ProductName" => p.product = Some(s),
                "FileDescription" => p.description = Some(s),
                "CompanyName" => p.company = Some(s),
                "ProductVersion" => p.product_version = Some(s),
                "FileVersion" => p.file_version = Some(s),
                "LegalCopyright" => p.copyright = Some(s),
                "OriginalFilename" => p.original_name = Some(s),
                _ => {}
            }
        }
    }
}

/// The program's own icon (its first icon group, as Explorer shows), the picture nearest to
/// `size` pixels and no smaller where there is one.
#[cfg(test)]
pub fn icon(data: &[u8], size: u32) -> Option<image::RgbaImage> {
    icon_of(&Pe::parse(data)?, size)
}

fn icon_of(pe: &Pe, size: u32) -> Option<image::RgbaImage> {
    let group = pe.resource(RT_GROUP_ICON, None)?;
    let count = u16_at(group, 4)? as usize;
    let mut best: Option<(u32, u32, u16)> = None; // (width, bits, id)
    for i in 0..count.min(64) {
        let e = 6 + i * 14;
        let w = match *group.get(e)? {
            0 => 256,
            w => w as u32,
        };
        let bits = u16_at(group, e + 6)? as u32;
        let id = u16_at(group, e + 12)?;
        let better = match best {
            None => true,
            Some((bw, bb, _)) => {
                let fits = |w: u32| w >= size;
                match (fits(w), fits(bw)) {
                    (true, false) => true,
                    (false, true) => false,
                    (true, true) => w < bw || (w == bw && bits > bb),
                    (false, false) => w > bw || (w == bw && bits > bb),
                }
            }
        };
        if better {
            best = Some((w, bits, id));
        }
    }
    let (_, _, id) = best?;
    let pic = pe.resource(RT_ICON, Some(id as u32))?;
    icon_picture(pic)
}

/// One icon picture from a program: a PNG, or a bitmap as in an .ico file.
fn icon_picture(pic: &[u8]) -> Option<image::RgbaImage> {
    if pic.starts_with(b"\x89PNG") {
        return image::load_from_memory_with_format(pic, image::ImageFormat::Png).ok().map(|i| i.into_rgba8());
    }
    // A bitmap: wrap it in a one-picture .ico so the image crate reads it, masks and all.
    let w = u32_at(pic, 4)?;
    let h = u32_at(pic, 8)? / 2;
    let bits = u16_at(pic, 14)?;
    let mut ico = Vec::with_capacity(22 + pic.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    ico.push(if w >= 256 { 0 } else { w as u8 });
    ico.push(if h >= 256 { 0 } else { h as u8 });
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&bits.to_le_bytes());
    ico.extend_from_slice(&(pic.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(pic);
    image::load_from_memory_with_format(&ico, image::ImageFormat::Ico).ok().map(|i| i.into_rgba8())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn utf16z(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()
    }

    fn pad4(v: &mut Vec<u8>) {
        while v.len() % 4 != 0 {
            v.push(0);
        }
    }

    /// A version-resource block: key, value (text or binary) and children.
    fn vblock(key: &str, value: &[u8], text: bool, children: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0, 0];
        let vlen = if text { value.len() / 2 } else { value.len() };
        b.extend_from_slice(&(vlen as u16).to_le_bytes());
        b.extend_from_slice(&(text as u16).to_le_bytes());
        b.extend_from_slice(&utf16z(key));
        pad4(&mut b);
        b.extend_from_slice(value);
        for c in children {
            pad4(&mut b);
            b.extend_from_slice(c);
        }
        let len = b.len() as u16;
        b[0..2].copy_from_slice(&len.to_le_bytes());
        b
    }

    /// A small 32-bit BGRA icon bitmap (as stored in an RT_ICON), all one colour.
    fn dib(w: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut d = Vec::new();
        for v in [40u32, w, w * 2] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0; 24]);
        for _ in 0..w * w {
            d.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        // The AND mask: rows padded to 4 bytes, all shown.
        d.extend(std::iter::repeat_n(0, (w.div_ceil(32) * 4 * w) as usize));
        d
    }

    /// A minimal 64-bit Windows program with a version resource and an icon group of two
    /// pictures (16 px red, 32 px blue).
    pub(crate) fn sample_exe() -> Vec<u8> {
        let strings = [("CompanyName", "Acme Ltd"), ("FileDescription", "Acme Paint"), ("ProductName", "Acme Paint Studio"), ("ProductVersion", "3.2.1"), ("LegalCopyright", "© 2026 Acme Ltd")];
        let table: Vec<Vec<u8>> = strings.iter().map(|(k, v)| vblock(k, &utf16z(v), true, &[])).collect();
        let mut fixed = Vec::new();
        for v in [0xfeef_04bdu32, 0x10000, (3 << 16) | 2, (1 << 16) | 77, (3 << 16) | 2, 1 << 16] {
            fixed.extend_from_slice(&v.to_le_bytes());
        }
        fixed.extend_from_slice(&[0; 28]);
        let version = vblock("VS_VERSION_INFO", &fixed, false, &[vblock("StringFileInfo", &[], true, &[vblock("040904b0", &[], true, &table)])]);
        let (small, big) = (dib(16, [255, 0, 0, 255]), dib(32, [0, 0, 255, 255]));
        let mut group = vec![0, 0, 1, 0, 2, 0];
        for (w, len, id) in [(16u8, small.len(), 1u16), (32, big.len(), 2)] {
            group.extend_from_slice(&[w, w, 0, 0, 1, 0, 32, 0]);
            group.extend_from_slice(&(len as u32).to_le_bytes());
            group.extend_from_slice(&id.to_le_bytes());
        }
        // The resource section: type → id → language → data, laid out by hand.
        const RVA: u32 = 0x1000;
        let leaves: Vec<(u32, u32, &[u8])> = vec![(RT_ICON, 1, &small), (RT_ICON, 2, &big), (RT_GROUP_ICON, 1, &group), (RT_VERSION, 1, &version)];
        let types: Vec<u32> = vec![RT_ICON, RT_GROUP_ICON, RT_VERSION];
        let dir = |n: usize| {
            let mut d = vec![0u8; 12];
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&(n as u16).to_le_bytes());
            d
        };
        // Sizes: root dir, one dir per type, one dir per leaf (language), data entries, data.
        let root_len = 16 + types.len() * 8;
        let type_dirs: Vec<usize> = types.iter().map(|t| 16 + leaves.iter().filter(|l| l.0 == *t).count() * 8).collect();
        let lang_len = 16 + 8;
        let mut at = root_len + type_dirs.iter().sum::<usize>();
        let lang_at: Vec<usize> = leaves.iter().map(|_| { let a = at; at += lang_len; a }).collect();
        let entry_at: Vec<usize> = leaves.iter().map(|_| { let a = at; at += 16; a }).collect();
        let mut data_at = Vec::new();
        for l in &leaves {
            data_at.push(at);
            at += l.2.len().div_ceil(8) * 8;
        }
        let mut rsrc = dir(types.len());
        let mut tat = root_len;
        for (t, len) in types.iter().zip(&type_dirs) {
            rsrc.extend_from_slice(&t.to_le_bytes());
            rsrc.extend_from_slice(&((tat as u32) | 0x8000_0000).to_le_bytes());
            tat += len;
        }
        for t in &types {
            let mine: Vec<usize> = (0..leaves.len()).filter(|&i| leaves[i].0 == *t).collect();
            rsrc.extend(dir(mine.len()));
            for i in mine {
                rsrc.extend_from_slice(&leaves[i].1.to_le_bytes());
                rsrc.extend_from_slice(&((lang_at[i] as u32) | 0x8000_0000).to_le_bytes());
            }
        }
        for i in 0..leaves.len() {
            rsrc.extend(dir(1));
            rsrc.extend_from_slice(&0x409u32.to_le_bytes());
            rsrc.extend_from_slice(&(entry_at[i] as u32).to_le_bytes());
        }
        for i in 0..leaves.len() {
            rsrc.extend_from_slice(&(RVA + data_at[i] as u32).to_le_bytes());
            rsrc.extend_from_slice(&(leaves[i].2.len() as u32).to_le_bytes());
            rsrc.extend_from_slice(&[0; 8]);
        }
        for l in &leaves {
            rsrc.extend_from_slice(l.2);
            while rsrc.len() % 8 != 0 {
                rsrc.push(0);
            }
        }
        // Headers: DOS stub, PE signature, COFF, PE32+ optional header, one section.
        let mut f = vec![0u8; 0x40];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        f.extend_from_slice(b"PE\0\0");
        let mut coff = vec![0u8; 20];
        coff[0..2].copy_from_slice(&0x8664u16.to_le_bytes());
        coff[2..4].copy_from_slice(&1u16.to_le_bytes());
        coff[16..18].copy_from_slice(&240u16.to_le_bytes());
        f.extend(coff);
        let mut opt = vec![0u8; 240];
        opt[0..2].copy_from_slice(&0x20bu16.to_le_bytes());
        opt[68..70].copy_from_slice(&2u16.to_le_bytes());
        opt[108..112].copy_from_slice(&16u32.to_le_bytes());
        opt[112 + 16..112 + 20].copy_from_slice(&RVA.to_le_bytes());
        opt[112 + 20..112 + 24].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        f.extend(opt);
        let raw = 0x200u32;
        let mut sec = vec![0u8; 40];
        sec[0..5].copy_from_slice(b".rsrc");
        sec[8..12].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        sec[12..16].copy_from_slice(&RVA.to_le_bytes());
        sec[16..20].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        sec[20..24].copy_from_slice(&raw.to_le_bytes());
        f.extend(sec);
        f.resize(raw as usize, 0);
        f.extend(rsrc);
        f
    }

    #[test]
    fn reads_version_and_arch() {
        let p = program(&sample_exe()).unwrap();
        assert_eq!(p.product.as_deref(), Some("Acme Paint Studio"));
        assert_eq!(p.company.as_deref(), Some("Acme Ltd"));
        assert_eq!(p.description.as_deref(), Some("Acme Paint"));
        assert_eq!(p.product_version.as_deref(), Some("3.2.1"));
        // No FileVersion string: the number from the fixed part.
        assert_eq!(p.file_version.as_deref(), Some("3.2.1.77"));
        assert_eq!(p.arch.as_deref(), Some("64-bit (x64)"));
        assert!(!p.console && !p.signed);
        assert!(program(b"MZ not really").is_none());
        assert!(program(b"\x7fELF").is_none());
    }

    #[test]
    fn picks_the_nearest_icon() {
        let exe = sample_exe();
        let big = icon(&exe, 32).unwrap();
        assert_eq!((big.width(), big.height()), (32, 32));
        assert_eq!(big.get_pixel(5, 5).0, [0, 0, 255, 255]);
        let small = icon(&exe, 16).unwrap();
        assert_eq!(small.width(), 16);
        assert_eq!(small.get_pixel(5, 5).0, [255, 0, 0, 255]);
        // Bigger than any: the biggest there is.
        assert_eq!(icon(&exe, 128).unwrap().width(), 32);
    }

    #[test]
    fn survives_damaged_programs() {
        let exe = sample_exe();
        for cut in [0x3c, 0x80, 0x150, 0x200, 0x260, exe.len() - 40] {
            let _ = program(&exe[..cut]);
            let _ = icon(&exe[..cut], 32);
        }
        let mut bad = exe.clone();
        for i in (0x200..bad.len()).step_by(7) {
            bad[i] = 0xff;
        }
        let _ = program(&bad);
        let _ = icon(&bad, 32);
    }
}

