//! Disk images, read-only: what's on a CD/DVD image (ISO 9660, with Joliet or Rock Ridge
//! names), and what virtual disks, Mac disk images and Windows images say about themselves
//! (size, type, partitions, editions). Nothing is mounted and nothing is written.

use crate::fsys::archive::{Index, Item};
use crate::help::doc::{Block, Span};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Most items listed from one disc.
const ITEMS_MAX: usize = 200_000;
/// Most bytes read for one directory's records.
const DIR_MAX: u64 = 16 << 20;

pub enum Disk {
    /// A disc whose files can be listed.
    Listing { index: Index, facts: Vec<(&'static str, String)> },
    /// A disk described by its headers.
    Details { blocks: Vec<Block>, facts: Vec<(&'static str, String)> },
}

pub fn read(path: &Path) -> Result<Disk, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let head = at(&mut f, 0, 4096);
    let tail = if len >= 512 { at(&mut f, len - 512, 512) } else { Vec::new() };
    if at(&mut f, 32769, 5) == b"CD001" {
        return iso(&mut f, len);
    }
    if head.starts_with(b"vhdxfile") {
        return Ok(vhdx(&mut f, &head, len));
    }
    if tail.starts_with(b"conectix") || head.starts_with(b"conectix") {
        let footer = if tail.starts_with(b"conectix") { &tail } else { &head[..512] };
        return Ok(vhd(&mut f, footer, len));
    }
    if head.starts_with(b"KDMV") {
        return Ok(vmdk_sparse(&mut f, &head, len));
    }
    if head.starts_with(b"# Disk DescriptorFile") {
        return Ok(vmdk_text(&String::from_utf8_lossy(&at(&mut f, 0, 64 << 10)), len));
    }
    if head.starts_with(b"QFI\xfb") {
        return Ok(qcow(&head, len));
    }
    if head.get(0x40..0x44) == Some(&0xBEDA_107Fu32.to_le_bytes()) {
        return Ok(vdi(&head, len));
    }
    if head.starts_with(b"MSWIM\0\0\0") {
        return Ok(wim(&mut f, &head, len));
    }
    if head.starts_with(b"encrcdsa") || head.starts_with(b"cdsaencr") {
        return Ok(details("Mac disk image", vec![("Format", "Encrypted Mac disk image (DMG)".into()), ("Size", crate::scan::human(len))], "It's encrypted, so nothing in it can be read without its password."));
    }
    if tail.starts_with(b"koly") {
        return Ok(dmg(&mut f, &tail, len));
    }
    if is_udf(&mut f) {
        return Ok(details("Disc image (UDF)", vec![("Format", "UDF disc image".into()), ("Size", crate::scan::human(len))], "This disc uses only the UDF file system, which Filescape can't list yet."));
    }
    // A raw disk: its partition table, if it has one.
    if let Some((scheme, parts)) = partitions(&mut f, 0, len) {
        let facts = vec![("Format", "Raw disk image".into()), ("Size", crate::scan::human(len)), ("Partitions", format!("{} ({scheme})", parts.len()))];
        let mut d = details("Raw disk image", facts, "A sector-by-sector copy of a disk. Filescape shows its partitions; it doesn't read the file systems inside them.");
        if let Disk::Details { blocks, .. } = &mut d {
            blocks.insert(blocks.len() - 2, part_table(&parts));
        }
        return Ok(d);
    }
    Err("Filescape doesn't recognise this disk image's format".into())
}

fn at(f: &mut File, off: u64, len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    if f.seek(SeekFrom::Start(off)).is_ok() {
        let _ = f.take(len as u64).read_to_end(&mut v);
    }
    v
}

fn le16(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
fn le32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn le64(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map_or(0, |s| u64::from_le_bytes(s.try_into().unwrap()))
}
fn be32(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
fn be64(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8).map_or(0, |s| u64::from_be_bytes(s.try_into().unwrap()))
}

/// Printable text from fixed-width, space- or zero-padded fields.
fn field(b: &[u8]) -> String {
    String::from_utf8_lossy(b).trim_matches(|c: char| c == ' ' || c == '\0').to_string()
}

fn utf16be(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&u).trim_matches(|c: char| c == ' ' || c == '\0').to_string()
}

fn utf16le(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&u).trim_matches(|c: char| c == ' ' || c == '\0').to_string()
}

fn plain(t: &str) -> Vec<Span> {
    vec![Span { text: t.to_string(), ..Default::default() }]
}

fn details(title: &str, facts: Vec<(&'static str, String)>, note: &str) -> Disk {
    let mut blocks = vec![Block::Heading { level: 1, spans: plain(title), id: String::new() }];
    for (k, v) in &facts {
        blocks.push(Block::Para(vec![Span { text: format!("{k}: "), bold: true, ..Default::default() }, Span { text: v.clone(), ..Default::default() }]));
    }
    blocks.push(Block::Rule);
    blocks.push(Block::Para(plain(note)));
    Disk::Details { blocks, facts }
}

/// "40 GB, of which 3.1 GB is stored in the file".
fn virtual_size(v: u64, file: u64) -> Vec<(&'static str, String)> {
    let mut out = vec![("Disk size", crate::scan::human(v))];
    if v > 0 && file < v {
        out.push(("Stored", format!("{} in the file ({}% of the disk)", crate::scan::human(file), (file as f64 / v as f64 * 100.0).round().max(1.0) as u64)));
    }
    out
}

// ---- ISO 9660 ----

/// An ISO 9660 date-time (7 bytes) as seconds since the Unix epoch.
fn iso_time(b: &[u8]) -> i64 {
    if b.len() < 7 || b[1] == 0 {
        return 0;
    }
    let off = (b[6] as i8) as i64 * 15 * 60;
    chrono::NaiveDate::from_ymd_opt(1900 + b[0] as i32, b[1] as u32, b[2] as u32)
        .and_then(|d| d.and_hms_opt(b[3] as u32, b[4] as u32, b[5] as u32))
        .map_or(0, |t| t.and_utc().timestamp() - off)
}

/// The 17-character date in a volume descriptor ("2024031512000000" plus offset).
fn iso_date_text(b: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(b.get(..14)?).ok()?;
    let t = chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d%H%M%S").ok()?;
    Some(t.format("%-d %b %Y").to_string())
}

/// The Rock Ridge name in a directory record's system use area, if it has one.
fn rock_ridge_name(su: &[u8]) -> Option<String> {
    let mut i = 0;
    let mut name = Vec::new();
    while i + 4 <= su.len() {
        let len = su[i + 2] as usize;
        if len < 4 || i + len > su.len() {
            break;
        }
        if &su[i..i + 2] == b"NM" && len >= 5 {
            name.extend_from_slice(&su[i + 5..i + len]);
        }
        i += len;
    }
    (!name.is_empty()).then(|| String::from_utf8_lossy(&name).to_string())
}

fn iso(f: &mut File, len: u64) -> Result<Disk, String> {
    let mut pvd = None;
    let mut joliet = None;
    let mut boot = false;
    let mut udf = false;
    for s in 16..64u64 {
        let d = at(f, s * 2048, 2048);
        if d.len() < 2048 {
            break;
        }
        match (&d[1..6], d[0]) {
            (b"CD001", 0) => boot |= d[7..30].starts_with(b"EL TORITO SPECIFICATION"),
            (b"CD001", 1) => pvd = pvd.or(Some(d)),
            (b"CD001", 2) if d[88] == b'%' && d[89] == b'/' && matches!(d[90], b'@' | b'C' | b'E') => joliet = joliet.or(Some(d)),
            (b"CD001", 255) => {}
            (b"NSR02" | b"NSR03", _) => udf = true,
            (b"BEA01" | b"TEA01" | b"BOOT2" | b"CDW02", _) => {}
            _ if d[0] == 255 => {}
            _ => break,
        }
    }
    let pvd = pvd.ok_or("No volume descriptor in it")?;
    let block = le16(&pvd, 128).max(512) as u64;
    let desc = joliet.as_ref().unwrap_or(&pvd);
    let root = &desc[156..190];
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut rock = false;
    let mut todo = vec![(le32(root, 2) as u64, le32(root, 10) as u64, String::new(), 0u32)];
    while let Some((extent, size, dir, depth)) = todo.pop() {
        if depth > 32 || !seen.insert(extent) || items.len() >= ITEMS_MAX {
            continue;
        }
        let data = at(f, extent * block, size.min(DIR_MAX) as usize);
        let mut i = 0;
        while i < data.len() && items.len() < ITEMS_MAX {
            let rl = data[i] as usize;
            if rl == 0 {
                // Records don't cross sectors: the rest of this one is padding.
                i = (i / 2048 + 1) * 2048;
                continue;
            }
            let Some(r) = data.get(i..i + rl) else { break };
            i += rl;
            if r.len() < 34 {
                continue;
            }
            let nl = r[32] as usize;
            let Some(raw) = r.get(33..33 + nl) else { continue };
            if nl == 1 && raw[0] <= 1 {
                continue; // "." and ".."
            }
            let is_dir = r[25] & 2 != 0;
            let su = r.get(33 + nl + (1 - nl % 2)..).unwrap_or(&[]);
            let mut name = if joliet.is_some() {
                utf16be(raw)
            } else if let Some(n) = rock_ridge_name(su) {
                rock = true;
                n
            } else {
                let n = String::from_utf8_lossy(raw).to_string();
                let n = n.split(';').next().unwrap_or("").to_string();
                n.strip_suffix('.').map(str::to_string).unwrap_or(n)
            };
            if joliet.is_some() {
                name = name.split(';').next().unwrap_or("").to_string();
            }
            // Never a name that climbs out or holds a separator.
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                continue;
            }
            let path = if dir.is_empty() { name } else { format!("{dir}/{name}") };
            let (ext, sz) = (le32(r, 2) as u64, le32(r, 10) as u64);
            // Multi-extent files repeat their record; count the size once per record anyway.
            items.push(Item { path: path.clone(), is_dir, size: if is_dir { 0 } else { sz }, modified: iso_time(&r[18..25]), packed: None });
            if is_dir {
                todo.push((ext, sz, path, depth + 1));
            }
        }
    }
    items.sort_by(|a, b| a.path.cmp(&b.path));
    items.dedup_by(|a, b| {
        if a.path == b.path && !a.is_dir {
            b.size += a.size;
            true
        } else {
            a.path == b.path
        }
    });
    let mut facts = vec![("Format", format!("Disc image (ISO 9660{})", if joliet.is_some() { ", Joliet names" } else if rock { ", Rock Ridge names" } else { "" }))];
    let label = if let Some(j) = &joliet { utf16be(&j[40..72]) } else { field(&pvd[40..72]) };
    if !label.is_empty() {
        facts.push(("Volume name", label));
    }
    let vsize = le32(&pvd, 80) as u64 * block;
    if vsize > 0 {
        facts.push(("Disc size", crate::scan::human(vsize)));
    }
    if let Some(d) = iso_date_text(&pvd[813..830]) {
        facts.push(("Made", d));
    }
    for (label, r) in [("Made with", 574..702), ("Publisher", 318..446)] {
        let v = field(&pvd[r]);
        if !v.is_empty() && !v.starts_with('_') {
            facts.push((label, v));
        }
    }
    if boot {
        facts.push(("Bootable", "Yes".into()));
    }
    let listed: u64 = items.iter().map(|i| i.size).sum();
    if udf && (listed < vsize / 10 || items.len() <= 2) {
        facts.push(("Note", "Its full contents are in a UDF file system, which Filescape can't list yet; what's shown is the small ISO 9660 part".into()));
    } else if udf {
        facts.push(("Also", "A UDF file system with the same files".into()));
    }
    if items.len() >= ITEMS_MAX {
        facts.push(("Note", format!("Only the first {} items are listed", crate::inspect::group(ITEMS_MAX as u64))));
    }
    let _ = len;
    Ok(Disk::Listing { index: Index { items, unsafe_names: 0, encrypted: 0 }, facts })
}

fn is_udf(f: &mut File) -> bool {
    (16..32u64).any(|s| matches!(at(f, s * 2048 + 1, 5).as_slice(), b"NSR02" | b"NSR03"))
}

// ---- Partition tables ----

struct Part {
    kind: String,
    name: String,
    start: u64,
    size: u64,
}

fn mbr_kind(t: u8) -> &'static str {
    match t {
        0x01 | 0x04 | 0x06 | 0x0E => "FAT",
        0x0B | 0x0C => "FAT32",
        0x07 => "NTFS or exFAT",
        0x05 | 0x0F | 0x85 => "Extended",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x8E => "Linux LVM",
        0xA5 => "FreeBSD",
        0xA8 => "macOS UFS",
        0xAB => "macOS boot",
        0xAF => "HFS+",
        0xEE => "GPT protective",
        0xEF => "EFI system",
        0xFD => "Linux RAID",
        _ => "Other",
    }
}

fn guid(b: &[u8]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{}",
        le32(b, 0),
        le16(b, 4),
        le16(b, 6),
        b[8],
        b[9],
        b[10..16].iter().map(|x| format!("{x:02X}")).collect::<String>()
    )
}

fn gpt_kind(g: &str) -> &'static str {
    match g {
        "C12A7328-F81F-11D2-BA4B-00A0C93EC93B" => "EFI system",
        "E3C9E316-0B5C-4DB8-817D-F92DF00215AE" => "Microsoft reserved",
        "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7" => "Windows data",
        "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC" => "Windows recovery",
        "0FC63DAF-8483-4772-8E79-3D69D8477DE4" => "Linux",
        "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F" => "Linux swap",
        "E6D6D379-F507-44C2-A23C-238F2A3DF928" => "Linux LVM",
        "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709" => "Linux root (x86-64)",
        "48465300-0000-11AA-AA11-00306543ECAC" => "HFS+",
        "7C3457EF-0000-11AA-AA11-00306543ECAC" => "APFS",
        "426F6F74-0000-11AA-AA11-00306543ECAC" => "macOS boot",
        "21686148-6449-6E6F-744E-656564454649" => "BIOS boot",
        _ => "Other",
    }
}

/// The partitions of a disk whose sector 0 is at `base` in the file.
fn partitions(f: &mut File, base: u64, len: u64) -> Option<(&'static str, Vec<Part>)> {
    let mbr = at(f, base, 512);
    if mbr.len() < 512 || mbr[510..512] != [0x55, 0xAA] {
        return None;
    }
    let gpt = at(f, base + 512, 512);
    if gpt.starts_with(b"EFI PART") {
        let (lba, count, esz) = (le64(&gpt, 72), le32(&gpt, 80).min(256) as u64, le32(&gpt, 84).max(128) as u64);
        let table = at(f, base + lba * 512, (count * esz) as usize);
        let mut parts = Vec::new();
        for e in table.chunks_exact(esz as usize) {
            if e[..16].iter().all(|&x| x == 0) {
                continue;
            }
            let (first, last) = (le64(e, 32), le64(e, 40));
            parts.push(Part { kind: gpt_kind(&guid(&e[..16])).into(), name: utf16le(&e[56..128]), start: first * 512, size: (last + 1).saturating_sub(first) * 512 });
        }
        return Some(("GPT", parts));
    }
    let mut parts = Vec::new();
    for i in 0..4 {
        let e = &mbr[446 + i * 16..462 + i * 16];
        if e[4] == 0 {
            continue;
        }
        let (start, n) = (le32(e, 8) as u64 * 512, le32(e, 12) as u64 * 512);
        if start >= len.max(1) && len > 0 && base == 0 {
            continue;
        }
        parts.push(Part { kind: mbr_kind(e[4]).into(), name: String::new(), start, size: n });
    }
    (!parts.is_empty()).then_some(("MBR", parts))
}

fn part_table(parts: &[Part]) -> Block {
    let head = ["Partition", "Kind", "Name", "Starts at", "Size"].iter().map(|h| plain(h)).collect();
    let rows = parts.iter().enumerate().map(|(i, p)| vec![plain(&(i + 1).to_string()), plain(&p.kind), plain(&p.name), plain(&crate::scan::human(p.start)), plain(&crate::scan::human(p.size))]).collect();
    Block::Table { head, rows }
}

// ---- Virtual disks ----

fn vhd(f: &mut File, footer: &[u8], len: u64) -> Disk {
    let kind = match be32(footer, 60) {
        2 => "Fixed size",
        3 => "Dynamically expanding",
        4 => "Differencing (changes on top of a parent disk)",
        _ => "Unknown",
    };
    let mut facts = vec![("Format", "Virtual hard disk (VHD)".into()), ("Type", kind.to_string())];
    facts.extend(virtual_size(be64(footer, 48), len));
    let app = match &footer[28..32] {
        b"vpc " => "Virtual PC".into(),
        b"vs  " => "Virtual Server".into(),
        b"win " => "Windows".into(),
        b"qemu" => "QEMU".into(),
        b"vbox" => "VirtualBox".into(),
        b"d2v " => "Disk2vhd".into(),
        a => field(a),
    };
    if !app.is_empty() {
        facts.push(("Made with", app));
    }
    let made = 946_684_800 + be32(footer, 24) as i64;
    if let Some(t) = chrono::DateTime::from_timestamp(made, 0) {
        facts.push(("Made", t.format("%-d %b %Y").to_string()));
    }
    let mut d = details("Virtual hard disk", facts, "A virtual machine's hard disk. Filescape shows what its header says; it doesn't read the file systems inside.");
    // A fixed disk's data is the file itself: show its partitions.
    if be32(footer, 60) == 2
        && let Some((scheme, parts)) = partitions(f, 0, len)
        && let Disk::Details { blocks, facts } = &mut d
    {
        facts.push(("Partitions", format!("{} ({scheme})", parts.len())));
        blocks.insert(blocks.len() - 2, part_table(&parts));
    }
    d
}

fn vhdx(f: &mut File, head: &[u8], len: u64) -> Disk {
    let mut facts = vec![("Format", "Virtual hard disk (VHDX)".into())];
    let creator = utf16le(&head[8..520.min(head.len())]);
    // The region table names the metadata region; its "virtual disk size" item has the size.
    let regions = at(f, 192 << 10, 64 << 10);
    let mut vsize = None;
    let mut differencing = false;
    let mut fixed = None;
    if regions.starts_with(b"regi") {
        let n = le32(&regions, 8).min(2047) as usize;
        for e in regions[16..].chunks_exact(32).take(n) {
            if guid(&e[..16]) != "8B7CA206-4790-4B9A-B8FE-575F050F886E" {
                continue;
            }
            let (off, rl) = (le64(e, 16), le32(e, 24) as usize);
            let m = at(f, off, rl.min(1 << 20));
            if !m.starts_with(b"metadata") {
                continue;
            }
            let count = le16(&m, 10) as usize;
            for it in m[32..].chunks_exact(32).take(count) {
                let (io, il) = (le32(it, 16) as usize, le32(it, 20) as usize);
                let Some(v) = m.get(io..io + il) else { continue };
                match guid(&it[..16]).as_str() {
                    "2FA54224-CD1B-4876-B211-5DBED83BF4B8" => vsize = Some(le64(v, 0)),
                    "CAA16737-FA36-4D43-B3B6-33F0AA44E76B" => {
                        let flags = le32(v, 4);
                        fixed = Some(flags & 1 != 0);
                        differencing = flags & 2 != 0;
                    }
                    _ => {}
                }
            }
        }
    }
    let kind = if differencing {
        "Differencing (changes on top of a parent disk)"
    } else {
        match fixed {
            Some(true) => "Fixed size",
            Some(false) => "Dynamically expanding",
            None => "",
        }
    };
    if !kind.is_empty() {
        facts.push(("Type", kind.into()));
    }
    if let Some(v) = vsize {
        facts.extend(virtual_size(v, len));
    } else {
        facts.push(("Size", crate::scan::human(len)));
    }
    if !creator.is_empty() {
        facts.push(("Made with", creator));
    }
    details("Virtual hard disk", facts, "A Hyper-V virtual machine's hard disk. Filescape shows what its header says; it doesn't read the file systems inside.")
}

fn vmdk_descriptor(text: &str, facts: &mut Vec<(&'static str, String)>) -> u64 {
    let mut sectors = 0u64;
    for line in text.lines() {
        let l = line.trim();
        let val = |k: &str| l.strip_prefix(k).map(|v| v.trim_start_matches([' ', '=']).trim_matches('"').to_string());
        if let Some(v) = val("createType") {
            let t = match v.as_str() {
                "monolithicSparse" | "streamOptimized" => "Grows as it's used, in one file",
                "monolithicFlat" => "Fixed size, in one file",
                "twoGbMaxExtentSparse" => "Grows as it's used, split into 2 GB files",
                "twoGbMaxExtentFlat" => "Fixed size, split into 2 GB files",
                "vmfs" | "vmfsThin" => "ESXi disk",
                _ => "",
            };
            facts.push(("Type", if t.is_empty() { v } else { t.into() }));
        } else if let Some(v) = val("ddb.adapterType") {
            facts.push(("Adapter", v));
        } else if let Some(v) = val("ddb.virtualHWVersion") {
            facts.push(("Hardware version", v));
        } else if let Some(v) = val("parentFileNameHint") {
            facts.push(("Parent disk", v));
        } else if l.starts_with("RW ") || l.starts_with("RDONLY ") {
            sectors += l.split_whitespace().nth(1).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
        }
    }
    sectors
}

fn vmdk_sparse(f: &mut File, head: &[u8], len: u64) -> Disk {
    let mut facts = vec![("Format", "VMware virtual disk (VMDK)".into())];
    let (cap, doff, dsize) = (le64(head, 12), le64(head, 28), le64(head, 36));
    if doff > 0 && dsize > 0 {
        let text = String::from_utf8_lossy(&at(f, doff * 512, (dsize * 512).min(1 << 20) as usize)).to_string();
        vmdk_descriptor(&text, &mut facts);
    }
    facts.extend(virtual_size(cap * 512, len));
    details("Virtual disk", facts, "A VMware virtual machine's hard disk. Filescape shows what its header says; it doesn't read the file systems inside.")
}

fn vmdk_text(text: &str, _len: u64) -> Disk {
    let mut facts = vec![("Format", "VMware disk descriptor (VMDK)".into())];
    let sectors = vmdk_descriptor(text, &mut facts);
    if sectors > 0 {
        facts.push(("Disk size", crate::scan::human(sectors * 512)));
    }
    let extents: Vec<String> = text.lines().filter(|l| l.starts_with("RW ") || l.starts_with("RDONLY ")).filter_map(|l| l.split('"').nth(1).map(str::to_string)).collect();
    let mut d = details("Virtual disk", facts, "This small file describes a VMware disk; the data is in the files listed.");
    if !extents.is_empty()
        && let Disk::Details { blocks, .. } = &mut d
    {
        blocks.insert(blocks.len() - 2, Block::List { start: None, items: extents.iter().map(|e| vec![Block::Para(plain(e))]).collect() });
    }
    d
}

fn qcow(head: &[u8], len: u64) -> Disk {
    let mut facts = vec![("Format", format!("QEMU disk image (qcow{})", be32(head, 4)))];
    facts.extend(virtual_size(be64(head, 24), len));
    let (boff, blen) = (be64(head, 8) as usize, be32(head, 16) as usize);
    if boff > 0
        && let Some(b) = head.get(boff..boff + blen)
    {
        facts.push(("Parent disk", String::from_utf8_lossy(b).to_string()));
    }
    if be32(head, 32) != 0 {
        facts.push(("Encrypted", "Yes".into()));
    }
    let snaps = be32(head, 60);
    if snaps > 0 {
        facts.push(("Snapshots", snaps.to_string()));
    }
    details("Virtual disk", facts, "A QEMU or KVM virtual machine's disk. Filescape shows what its header says; it doesn't read the file systems inside.")
}

fn vdi(head: &[u8], len: u64) -> Disk {
    let mut facts = vec![("Format", "VirtualBox disk image (VDI)".into())];
    let kind = match le32(head, 0x4C) {
        1 => "Dynamically expanding",
        2 => "Fixed size",
        4 => "Differencing (changes on top of a parent disk)",
        _ => "",
    };
    if !kind.is_empty() {
        facts.push(("Type", kind.into()));
    }
    facts.extend(virtual_size(le64(head, 0x170), len));
    details("Virtual disk", facts, "A VirtualBox virtual machine's disk. Filescape shows what its header says; it doesn't read the file systems inside.")
}

// ---- Mac and Windows images ----

fn dmg(f: &mut File, koly: &[u8], len: u64) -> Disk {
    let mut facts = vec![("Format", "Mac disk image (DMG)".into())];
    let sectors = be64(koly, 0x1EC);
    facts.extend(virtual_size(sectors * 512, len));
    let (xoff, xlen) = (be64(koly, 0xD8), be64(koly, 0xE0));
    let mut parts = Vec::new();
    if xlen > 0 && xlen < 64 << 20 {
        let xml = String::from_utf8_lossy(&at(f, xoff, xlen as usize)).to_string();
        // Each partition's entry in the plist's blkx array has a Name.
        let mut rest = xml.as_str();
        while let Some(i) = rest.find("<key>Name</key>") {
            rest = &rest[i + 15..];
            if let Some(s) = rest.find("<string>") {
                let v = &rest[s + 8..];
                if let Some(e) = v.find("</string>") {
                    parts.push(v[..e].trim().to_string());
                }
            }
        }
    }
    let kinds: Vec<&str> = parts
        .iter()
        .filter_map(|p| {
            let p = p.to_ascii_lowercase();
            if p.contains("apfs") {
                Some("APFS")
            } else if p.contains("apple_hfs") || p.contains("hfs") {
                Some("HFS+")
            } else if p.contains("fat") || p.contains("dos") {
                Some("FAT")
            } else {
                None
            }
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if !kinds.is_empty() {
        facts.push(("File system", kinds.join(", ")));
    }
    let mut d = details("Mac disk image", facts, "Open it on a Mac to see what's inside. Filescape shows what the image says about itself.");
    if !parts.is_empty()
        && let Disk::Details { blocks, .. } = &mut d
    {
        blocks.insert(blocks.len() - 2, Block::Heading { level: 2, spans: plain("Partitions"), id: String::new() });
        blocks.insert(blocks.len() - 2, Block::List { start: None, items: parts.iter().map(|p| vec![Block::Para(plain(p))]).collect() });
    }
    d
}

fn wim(f: &mut File, head: &[u8], len: u64) -> Disk {
    let count = le32(head, 44);
    let mut facts = vec![("Format", "Windows image (WIM/ESD)".into()), ("Size", crate::scan::human(len))];
    if count > 0 {
        facts.push(("Images", count.to_string()));
    }
    // The XML resource header: size (7 bytes) and flags, offset, original size.
    let (xsize, xoff) = (le64(head, 72) & 0x00FF_FFFF_FFFF_FFFF, le64(head, 80));
    let mut rows = Vec::new();
    if xsize > 0 && xsize < 16 << 20 {
        let xml = utf16le(&at(f, xoff, xsize as usize));
        let tag = |s: &str, t: &str| -> Option<String> {
            let a = s.find(&format!("<{t}>"))? + t.len() + 2;
            let b = s[a..].find(&format!("</{t}>"))? + a;
            Some(s[a..b].trim().to_string())
        };
        for img in xml.split("<IMAGE ").skip(1) {
            let img = img.split("</IMAGE>").next().unwrap_or("");
            let name = tag(img, "DISPLAYNAME").or_else(|| tag(img, "NAME")).unwrap_or_default();
            let bytes = tag(img, "TOTALBYTES").and_then(|b| b.parse::<u64>().ok()).unwrap_or(0);
            let ver = match (tag(img, "MAJOR"), tag(img, "MINOR"), tag(img, "BUILD")) {
                (Some(a), Some(b), Some(c)) => format!("{a}.{b}.{c}"),
                _ => String::new(),
            };
            rows.push(vec![plain(&name), plain(&ver), plain(&if bytes > 0 { crate::scan::human(bytes) } else { String::new() })]);
        }
    }
    if let Some(first) = rows.first()
        && let Some(v) = first[1].first().map(|s| s.text.clone()).filter(|v| !v.is_empty())
    {
        facts.push(("Windows version", v));
    }
    let mut d = details("Windows image", facts, "A Windows installation image. Each image inside is one edition that can be installed.");
    if !rows.is_empty()
        && let Disk::Details { blocks, .. } = &mut d
    {
        let head = ["Edition", "Version", "Installed size"].iter().map(|h| plain(h)).collect();
        blocks.insert(blocks.len() - 2, Block::Table { head, rows });
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("fs-disk-{}-{name}", std::process::id()));
        std::fs::write(&d, bytes).unwrap();
        d
    }

    fn record(name: &[u8], extent: u32, size: u32, dir: bool) -> Vec<u8> {
        let nl = name.len();
        let rl = 33 + nl + (1 - nl % 2);
        let mut r = vec![0u8; rl];
        r[0] = rl as u8;
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[18..25].copy_from_slice(&[124, 3, 15, 12, 0, 0, 0]);
        r[25] = if dir { 2 } else { 0 };
        r[32] = nl as u8;
        r[33..33 + nl].copy_from_slice(name);
        r
    }

    /// A tiny ISO: root (sector 20) with README.TXT;1 and DOCS, DOCS (sector 21) with A.PDF;1.
    fn tiny_iso() -> Vec<u8> {
        let mut b = vec![0u8; 2048 * 24];
        let pvd = &mut b[16 * 2048..17 * 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[40..48].copy_from_slice(b"MY_DISC ");
        pvd[80..84].copy_from_slice(&24u32.to_le_bytes());
        pvd[128..130].copy_from_slice(&2048u16.to_le_bytes());
        pvd[156..190].copy_from_slice(&record(&[0], 20, 2048, true));
        pvd[813..830].copy_from_slice(b"20240315120000000");
        let term = &mut b[17 * 2048..18 * 2048];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        let mut root = Vec::new();
        root.extend(record(&[0], 20, 2048, true));
        root.extend(record(&[1], 20, 2048, true));
        root.extend(record(b"README.TXT;1", 22, 1234, false));
        root.extend(record(b"DOCS", 21, 2048, true));
        b[20 * 2048..20 * 2048 + root.len()].copy_from_slice(&root);
        let mut docs = Vec::new();
        docs.extend(record(&[0], 21, 2048, true));
        docs.extend(record(&[1], 20, 2048, true));
        docs.extend(record(b"A.PDF;1", 23, 99_000, false));
        b[21 * 2048..21 * 2048 + docs.len()].copy_from_slice(&docs);
        b
    }

    #[test]
    fn lists_an_iso() {
        let p = tmp("a.iso", &tiny_iso());
        let Ok(Disk::Listing { index, facts }) = read(&p) else { panic!("not listed") };
        let paths: Vec<_> = index.items.iter().map(|i| (i.path.as_str(), i.is_dir, i.size)).collect();
        assert_eq!(paths, vec![("DOCS", true, 0), ("DOCS/A.PDF", false, 99_000), ("README.TXT", false, 1234)]);
        assert!(facts.iter().any(|f| f == &("Volume name", "MY_DISC".to_string())));
        assert!(facts.iter().any(|f| f == &("Made", "15 Mar 2024".to_string())));
        assert!(index.items[2].modified > 0);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn describes_virtual_disks() {
        let mut q = vec![0u8; 512];
        q[..4].copy_from_slice(b"QFI\xfb");
        q[4..8].copy_from_slice(&3u32.to_be_bytes());
        q[24..32].copy_from_slice(&(20u64 << 30).to_be_bytes());
        let p = tmp("a.qcow2", &q);
        let Ok(Disk::Details { facts, .. }) = read(&p) else { panic!() };
        assert!(facts.iter().any(|f| f.0 == "Disk size" && f.1.contains("20")), "{facts:?}");
        assert!(facts.iter().any(|f| f == &("Format", "QEMU disk image (qcow3)".to_string())));
        let _ = std::fs::remove_file(p);

        // A fixed VHD: a disk with an MBR, then the footer.
        let mut v = vec![0u8; 4096 + 512];
        v[510] = 0x55;
        v[511] = 0xAA;
        v[446 + 4] = 0x07;
        v[446 + 8..446 + 12].copy_from_slice(&1u32.to_le_bytes());
        v[446 + 12..446 + 16].copy_from_slice(&7u32.to_le_bytes());
        let f = &mut v[4096..];
        f[..8].copy_from_slice(b"conectix");
        f[28..32].copy_from_slice(b"win ");
        f[48..56].copy_from_slice(&4096u64.to_be_bytes());
        f[60..64].copy_from_slice(&2u32.to_be_bytes());
        let p = tmp("a.vhd", &v);
        let Ok(Disk::Details { facts, blocks }) = read(&p) else { panic!() };
        assert!(facts.iter().any(|f| f == &("Type", "Fixed size".to_string())));
        assert!(facts.iter().any(|f| f == &("Partitions", "1 (MBR)".to_string())), "{facts:?}");
        assert!(blocks.iter().any(|b| matches!(b, Block::Table { rows, .. } if rows.len() == 1)));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn rock_ridge_names() {
        let mut su = b"PX\x0c\x01aaaaaaaa".to_vec();
        su.extend(b"NM\x0e\x01\x00long-name");
        assert_eq!(rock_ridge_name(&su).as_deref(), Some("long-name"));
    }

    /// FILESCAPE_DISK_DIR=… cargo test --release disk_sample_folder -- --ignored --nocapture
    #[test]
    #[ignore]
    fn disk_sample_folder() {
        let dir = std::env::var("FILESCAPE_DISK_DIR").expect("FILESCAPE_DISK_DIR");
        let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names {
            match read(&p) {
                Ok(Disk::Listing { index, facts }) => {
                    println!("list {:?} {facts:?}", p.file_name().unwrap());
                    for i in &index.items {
                        println!("     {} {} {}", i.path, i.is_dir, i.size);
                    }
                }
                Ok(Disk::Details { facts, blocks }) => println!("info {:?} {facts:?} blocks={}", p.file_name().unwrap(), blocks.len()),
                Err(e) => println!("err  {:?} {e}", p.file_name().unwrap()),
            }
        }
    }
}
