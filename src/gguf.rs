//! Reading the header of a GGUF file (llama.cpp's model format): its metadata and its tensor
//! directory, without reading the weights.
//!
//! Layout: "GGUF", version (u32), tensor count (u64), metadata count (u64), the metadata
//! (key, type, value), then one entry per tensor (name, dimensions, type, offset), then padding
//! to `general.alignment` (32 by default) and the tensor data. Every count and length is checked
//! before it is trusted, so a damaged or hostile file gives an error, never a huge allocation.

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Longest string, and most metadata entries or tensors, a header may have.
const MAX_STR: u64 = 1 << 20;
const MAX_ENTRIES: u64 = 1 << 20;
/// Arrays up to this length are kept; longer ones (vocabularies) only have their length kept.
const KEEP_ARRAY: u64 = 64;

/// A metadata value. Long arrays keep only their type and length.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Uint(u64),
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Array(Vec<Value>),
    /// An array too long to keep: (item type name, length).
    LongArray(&'static str, u64),
}

impl Value {
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Value::Uint(v) => Some(v),
            Value::Int(v) => u64::try_from(v).ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Number of items, for arrays.
    pub fn len(&self) -> Option<u64> {
        match self {
            Value::Array(v) => Some(v.len() as u64),
            Value::LongArray(_, n) => Some(*n),
            _ => None,
        }
    }

    /// Short text for display: long strings and arrays are summarised.
    pub fn show(&self) -> String {
        match self {
            Value::Uint(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::Float(v) => format!("{v}"),
            Value::Bool(v) => v.to_string(),
            Value::Str(s) if s.chars().count() > 200 => format!("{}… ({} characters)", s.chars().take(200).collect::<String>(), s.chars().count()),
            Value::Str(s) => s.clone(),
            Value::Array(v) if v.len() <= 8 => format!("[{}]", v.iter().map(Value::show).collect::<Vec<_>>().join(", ")),
            Value::Array(v) => format!("[{}, … {} items]", v.iter().take(4).map(Value::show).collect::<Vec<_>>().join(", "), v.len()),
            Value::LongArray(t, n) => format!("{n} {t} values"),
        }
    }
}

/// One tensor in the directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub name: String,
    /// Dimensions as GGUF stores them (fastest-changing first).
    pub dims: Vec<u64>,
    /// ggml type number (see [`type_name`]).
    pub kind: u32,
    /// Where the data starts, from the start of the file.
    pub offset: u64,
    /// Bytes of data, from the type's block size.
    pub bytes: u64,
}

impl Tensor {
    pub fn elements(&self) -> u64 {
        self.dims.iter().product()
    }
}

/// A file's header.
#[derive(Clone, Debug, Default)]
pub struct Header {
    pub version: u32,
    pub meta: Vec<(String, Value)>,
    pub tensors: Vec<Tensor>,
    /// Where the tensor data starts.
    pub data_start: u64,
}

impl Header {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.meta.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(Value::as_u64)
    }

    /// `<architecture>.<key>`, such as `llama.block_count`.
    pub fn arch_u64(&self, key: &str) -> Option<u64> {
        let arch = self.str("general.architecture")?;
        self.u64(&format!("{arch}.{key}"))
    }
}

/// Why a header couldn't be read.
#[derive(Debug, PartialEq)]
pub enum Error {
    NotGguf,
    Version(u32),
    /// The file ends inside the header.
    Truncated,
    /// A count or length is impossible (damaged file).
    Damaged(&'static str),
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NotGguf => write!(f, "not a GGUF file"),
            Error::Version(v) => write!(f, "GGUF version {v} isn't supported"),
            Error::Truncated => write!(f, "the file ends inside its header (incomplete download?)"),
            Error::Damaged(what) => write!(f, "the header is damaged ({what})"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Reads the header. With `tensors` false, stops after the metadata (quicker for a list).
pub fn read(path: &Path, tensors: bool) -> Result<Header, Error> {
    let f = std::fs::File::open(path).map_err(|e| Error::Io(e.to_string()))?;
    read_from(BufReader::with_capacity(256 * 1024, f), tensors)
}

pub fn read_from<R: Read + Seek>(mut r: BufReader<R>, tensors: bool) -> Result<Header, Error> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).map_err(|_| Error::NotGguf)?;
    if &magic != b"GGUF" {
        return Err(Error::NotGguf);
    }
    let version = u32_(&mut r)?;
    if !(2..=3).contains(&version) {
        return Err(Error::Version(version));
    }
    let n_tensors = u64_(&mut r)?;
    let n_meta = u64_(&mut r)?;
    if n_tensors > MAX_ENTRIES || n_meta > MAX_ENTRIES {
        return Err(Error::Damaged("impossible number of entries"));
    }
    let mut h = Header { version, ..Default::default() };
    for _ in 0..n_meta {
        let key = string(&mut r)?;
        let ty = u32_(&mut r)?;
        let v = value(&mut r, ty, 0)?;
        h.meta.push((key, v));
    }
    if !tensors {
        return Ok(h);
    }
    let mut raw = Vec::with_capacity(n_tensors.min(100_000) as usize);
    for _ in 0..n_tensors {
        let name = string(&mut r)?;
        let n_dims = u32_(&mut r)?;
        if n_dims > 8 {
            return Err(Error::Damaged("too many dimensions"));
        }
        let mut dims = Vec::with_capacity(n_dims as usize);
        for _ in 0..n_dims {
            dims.push(u64_(&mut r)?);
        }
        let kind = u32_(&mut r)?;
        let offset = u64_(&mut r)?;
        raw.push((name, dims, kind, offset));
    }
    let end = r.stream_position().map_err(|e| Error::Io(e.to_string()))?;
    let align = h.u64("general.alignment").filter(|a| *a > 0 && a.is_power_of_two()).unwrap_or(32);
    h.data_start = end.div_ceil(align) * align;
    for (name, dims, kind, offset) in raw {
        let elements = dims.iter().try_fold(1u64, |a, &d| a.checked_mul(d)).ok_or(Error::Damaged("tensor too large"))?;
        let bytes = tensor_bytes(kind, elements).unwrap_or(0);
        let offset = h.data_start.checked_add(offset).ok_or(Error::Damaged("tensor offset"))?;
        h.tensors.push(Tensor { name, dims, kind, offset, bytes });
    }
    Ok(h)
}

fn u32_(r: &mut impl Read) -> Result<u32, Error> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).map_err(|_| Error::Truncated)?;
    Ok(u32::from_le_bytes(b))
}

fn u64_(r: &mut impl Read) -> Result<u64, Error> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).map_err(|_| Error::Truncated)?;
    Ok(u64::from_le_bytes(b))
}

fn string(r: &mut impl Read) -> Result<String, Error> {
    let n = u64_(r)?;
    if n > MAX_STR {
        return Err(Error::Damaged("string too long"));
    }
    let mut b = vec![0u8; n as usize];
    r.read_exact(&mut b).map_err(|_| Error::Truncated)?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

fn skip_string<R: Read + Seek>(r: &mut BufReader<R>) -> Result<(), Error> {
    let n = u64_(r)?;
    if n > MAX_STR {
        return Err(Error::Damaged("string too long"));
    }
    r.seek_relative(n as i64).map_err(|_| Error::Truncated)
}

/// Bytes of a fixed-size value type.
fn scalar_size(ty: u32) -> Option<u64> {
    Some(match ty {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        _ => return None,
    })
}

fn type_label(ty: u32) -> &'static str {
    match ty {
        0 => "u8",
        1 => "i8",
        2 => "u16",
        3 => "i16",
        4 => "u32",
        5 => "i32",
        6 => "f32",
        7 => "bool",
        8 => "string",
        9 => "array",
        10 => "u64",
        11 => "i64",
        12 => "f64",
        _ => "unknown",
    }
}

fn value<R: Read + Seek>(r: &mut BufReader<R>, ty: u32, depth: u32) -> Result<Value, Error> {
    let mut b = [0u8; 8];
    let mut fixed = |r: &mut BufReader<R>, n: usize| -> Result<[u8; 8], Error> {
        b = [0u8; 8];
        r.read_exact(&mut b[..n]).map_err(|_| Error::Truncated)?;
        Ok(b)
    };
    Ok(match ty {
        0 => Value::Uint(fixed(r, 1)?[0] as u64),
        1 => Value::Int(fixed(r, 1)?[0] as i8 as i64),
        2 => Value::Uint(u16::from_le_bytes(fixed(r, 2)?[..2].try_into().unwrap()) as u64),
        3 => Value::Int(i16::from_le_bytes(fixed(r, 2)?[..2].try_into().unwrap()) as i64),
        4 => Value::Uint(u32::from_le_bytes(fixed(r, 4)?[..4].try_into().unwrap()) as u64),
        5 => Value::Int(i32::from_le_bytes(fixed(r, 4)?[..4].try_into().unwrap()) as i64),
        6 => Value::Float(f32::from_le_bytes(fixed(r, 4)?[..4].try_into().unwrap()) as f64),
        7 => Value::Bool(fixed(r, 1)?[0] != 0),
        8 => Value::Str(string(r)?),
        10 => Value::Uint(u64::from_le_bytes(fixed(r, 8)?)),
        11 => Value::Int(i64::from_le_bytes(fixed(r, 8)?)),
        12 => Value::Float(f64::from_le_bytes(fixed(r, 8)?)),
        9 => {
            if depth > 2 {
                return Err(Error::Damaged("arrays nested too deep"));
            }
            let item = u32_(r)?;
            let n = u64_(r)?;
            if n > 1 << 32 {
                return Err(Error::Damaged("array too long"));
            }
            if n <= KEEP_ARRAY {
                let mut v = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    v.push(value(r, item, depth + 1)?);
                }
                Value::Array(v)
            } else {
                match item {
                    8 => {
                        for _ in 0..n {
                            skip_string(r)?;
                        }
                    }
                    9 => {
                        for _ in 0..n {
                            value(r, 9, depth + 1)?;
                        }
                    }
                    t => {
                        let size = scalar_size(t).ok_or(Error::Damaged("unknown value type"))?;
                        let bytes = size.checked_mul(n).ok_or(Error::Damaged("array too long"))?;
                        r.seek(SeekFrom::Current(bytes as i64)).map_err(|_| Error::Truncated)?;
                    }
                }
                Value::LongArray(type_label(item), n)
            }
        }
        _ => return Err(Error::Damaged("unknown value type")),
    })
}

/// (elements per block, bytes per block) of a ggml type.
pub fn block(kind: u32) -> Option<(u64, u64)> {
    Some(match kind {
        0 => (1, 4),
        1 => (1, 2),
        2 => (32, 18),
        3 => (32, 20),
        6 => (32, 22),
        7 => (32, 24),
        8 => (32, 34),
        9 => (32, 40),
        10 => (256, 84),
        11 => (256, 110),
        12 => (256, 144),
        13 => (256, 176),
        14 => (256, 210),
        15 => (256, 292),
        16 => (256, 66),
        17 => (256, 74),
        18 => (256, 98),
        19 => (256, 50),
        20 => (32, 18),
        21 => (256, 110),
        22 => (256, 82),
        23 => (256, 136),
        24 => (1, 1),
        25 => (1, 2),
        26 => (1, 4),
        27 => (1, 8),
        28 => (1, 8),
        29 => (256, 56),
        30 => (1, 2),
        34 => (256, 54),
        35 => (256, 66),
        39 => (32, 17),
        40 => (64, 36),
        41 => (128, 18),
        _ => return None,
    })
}

/// Bytes a tensor of this type and element count takes, if the type is known.
pub fn tensor_bytes(kind: u32, elements: u64) -> Option<u64> {
    let (per, size) = block(kind)?;
    elements.div_ceil(per).checked_mul(size)
}

/// The type's name, as llama.cpp prints it.
pub fn type_name(kind: u32) -> &'static str {
    match kind {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        6 => "Q5_0",
        7 => "Q5_1",
        8 => "Q8_0",
        9 => "Q8_1",
        10 => "Q2_K",
        11 => "Q3_K",
        12 => "Q4_K",
        13 => "Q5_K",
        14 => "Q6_K",
        15 => "Q8_K",
        16 => "IQ2_XXS",
        17 => "IQ2_XS",
        18 => "IQ3_XXS",
        19 => "IQ1_S",
        20 => "IQ4_NL",
        21 => "IQ3_S",
        22 => "IQ2_S",
        23 => "IQ4_XS",
        24 => "I8",
        25 => "I16",
        26 => "I32",
        27 => "I64",
        28 => "F64",
        29 => "IQ1_M",
        30 => "BF16",
        34 => "TQ1_0",
        35 => "TQ2_0",
        39 => "MXFP4",
        40 => "NVFP4",
        41 => "Q1_0",
        _ => "unknown",
    }
}

/// Writes a GGUF header for tests: metadata (strings and u32s), a long string array that must
/// be skipped, then tensors. Returns where the data starts.
#[cfg(test)]
pub(crate) fn write_test(path: &Path, strings: &[(&str, &str)], numbers: &[(&str, u32)], tensors: &[(&str, &[u64], u32)], pad: u64) -> u64 {
    use std::io::Write;
    let mut b = Vec::new();
    let s = |b: &mut Vec<u8>, x: &str| {
        b.extend((x.len() as u64).to_le_bytes());
        b.extend(x.as_bytes());
    };
    b.extend(b"GGUF");
    b.extend(3u32.to_le_bytes());
    b.extend((tensors.len() as u64).to_le_bytes());
    b.extend(((strings.len() + numbers.len() + 1) as u64).to_le_bytes());
    s(&mut b, "tokenizer.ggml.tokens");
    b.extend(9u32.to_le_bytes());
    b.extend(8u32.to_le_bytes());
    b.extend(100u64.to_le_bytes());
    for i in 0..100 {
        s(&mut b, &format!("tok{i}"));
    }
    for (k, v) in strings {
        s(&mut b, k);
        b.extend(8u32.to_le_bytes());
        s(&mut b, v);
    }
    for (k, v) in numbers {
        s(&mut b, k);
        b.extend(4u32.to_le_bytes());
        b.extend(v.to_le_bytes());
    }
    let mut off = 0u64;
    for (name, dims, kind) in tensors {
        s(&mut b, name);
        b.extend((dims.len() as u32).to_le_bytes());
        for d in *dims {
            b.extend(d.to_le_bytes());
        }
        b.extend(kind.to_le_bytes());
        b.extend(off.to_le_bytes());
        let bytes = tensor_bytes(*kind, dims.iter().product()).unwrap();
        off += bytes.div_ceil(32) * 32;
    }
    let start = (b.len() as u64).div_ceil(32) * 32;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&b).unwrap();
    // Sparse data, so tests use no disk.
    f.set_len(start + off + pad).unwrap();
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_metadata_and_tensors() {
        let p = std::env::temp_dir().join(format!("filescape-gguf-{}.gguf", std::process::id()));
        let start = write_test(
            &p,
            &[("general.architecture", "llama"), ("general.name", "Tiny")],
            &[("llama.block_count", 2), ("general.file_type", 15)],
            &[("token_embd.weight", &[64, 100], 12), ("blk.0.attn_q.weight", &[64, 64], 8), ("output_norm.weight", &[64], 0)],
            0,
        );
        let h = read(&p, true).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(h.version, 3);
        assert_eq!(h.str("general.name"), Some("Tiny"));
        assert_eq!(h.arch_u64("block_count"), Some(2));
        assert_eq!(h.get("tokenizer.ggml.tokens"), Some(&Value::LongArray("string", 100)));
        assert_eq!(h.data_start, start);
        assert_eq!(h.tensors.len(), 3);
        // Q4_K: 6400 elements = 25 blocks of 144 bytes.
        assert_eq!(h.tensors[0].bytes, 25 * 144);
        assert_eq!(h.tensors[1].elements(), 4096);
        assert_eq!(h.tensors[1].bytes, 128 * 34);
        assert_eq!(h.tensors[0].offset, start);
        assert_eq!(type_name(h.tensors[2].kind), "F32");
    }

    #[test]
    fn damaged_headers_are_errors() {
        let ok = |b: &[u8]| read_from(BufReader::new(std::io::Cursor::new(b.to_vec())), true);
        assert_eq!(ok(b"GGML").unwrap_err(), Error::NotGguf);
        assert_eq!(ok(b"GGUF\x03\0\0\0").unwrap_err(), Error::Truncated);
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(u64::MAX.to_le_bytes());
        assert!(matches!(ok(&b).unwrap_err(), Error::Damaged(_)));
        // A string claiming to be 1 TB long.
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(1u64.to_le_bytes());
        b.extend((1u64 << 40).to_le_bytes());
        assert!(matches!(ok(&b).unwrap_err(), Error::Damaged(_)));
    }
}
