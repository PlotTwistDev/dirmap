//! Serial numbers: `FST-XXXXX-XXXXX-XXXXX-XXXXX` (temporary) or `FSP-…` (perpetual).
//!
//! Crockford base32 (no I, L, O or U). 16 characters carry 80 random bits and the last 4 carry
//! 20 bits of a SHA-256 check over the prefix and body, so a mistyped serial is caught in the app
//! and never reaches the server, where a failed attempt could look like abuse. (Failed attempts
//! never consume a use either way.)
//!
//! The server stores only [`hash`], never the serial itself.

use crate::model::LicenceKind;
use ring::rand::SecureRandom;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const BODY: usize = 16;
const CHECK: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerialError {
    /// Not the right shape: wrong prefix, length or characters.
    Malformed,
    /// The right shape, but the check characters don't match: almost certainly a typo.
    Typo,
}

impl std::fmt::Display for SerialError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            SerialError::Malformed => "That doesn't look like a Filescape serial number (FST-… or FSP-…).",
            SerialError::Typo => "That serial number has a typo. Check it against your email or receipt.",
        })
    }
}

fn prefix(kind: LicenceKind) -> &'static str {
    match kind {
        LicenceKind::Temporary => "FST",
        LicenceKind::Perpetual => "FSP",
    }
}

fn check_chars(prefix: &str, body: &[u8]) -> [u8; CHECK] {
    let h = crate::sha256(&[b"filescape-serial-check\0", prefix.as_bytes(), body]);
    let bits = (u32::from(h[0]) << 12) | (u32::from(h[1]) << 4) | (u32::from(h[2]) >> 4);
    let mut out = [0u8; CHECK];
    for (i, c) in out.iter_mut().enumerate() {
        *c = ALPHABET[((bits >> (5 * (CHECK - 1 - i))) & 31) as usize];
    }
    out
}

fn format(prefix: &str, chars: &[u8]) -> String {
    let mut s = String::from(prefix);
    for group in chars.chunks(5) {
        s.push('-');
        s.push_str(std::str::from_utf8(group).unwrap());
    }
    s
}

/// A new random serial of the given kind.
pub fn generate(kind: LicenceKind, rng: &dyn SecureRandom) -> String {
    let mut raw = [0u8; 10];
    rng.fill(&mut raw).expect("system random number generator failed");
    let mut acc: u128 = 0;
    for b in raw {
        acc = (acc << 8) | u128::from(b);
    }
    let mut chars = Vec::with_capacity(BODY + CHECK);
    for i in (0..BODY).rev() {
        chars.push(ALPHABET[((acc >> (5 * i)) & 31) as usize]);
    }
    let p = prefix(kind);
    let check = check_chars(p, &chars);
    chars.extend_from_slice(&check);
    format(p, &chars)
}

/// Reads a serial as a person might type or paste it (any case, spaces or dashes anywhere,
/// O for 0 and I or L for 1) and returns its kind and canonical form.
pub fn normalize(input: &str) -> Result<(LicenceKind, String), SerialError> {
    let mut clean: Vec<u8> = Vec::new();
    for c in input.chars() {
        if c.is_whitespace() || c == '-' || c == '–' {
            continue;
        }
        if !c.is_ascii_alphanumeric() {
            return Err(SerialError::Malformed);
        }
        clean.push(c.to_ascii_uppercase() as u8);
    }
    if clean.len() != 3 + BODY + CHECK {
        return Err(SerialError::Malformed);
    }
    let kind = match &clean[..3] {
        b"FST" => LicenceKind::Temporary,
        b"FSP" => LicenceKind::Perpetual,
        _ => return Err(SerialError::Malformed),
    };
    let mut chars = Vec::with_capacity(BODY + CHECK);
    for &c in &clean[3..] {
        let c = match c {
            b'O' => b'0',
            b'I' | b'L' => b'1',
            c => c,
        };
        if !ALPHABET.contains(&c) {
            return Err(SerialError::Malformed);
        }
        chars.push(c);
    }
    let p = prefix(kind);
    if check_chars(p, &chars[..BODY]) != chars[BODY..] {
        return Err(SerialError::Typo);
    }
    Ok((kind, format(p, &chars)))
}

/// What the server stores and looks serials up by.
pub fn hash(canonical: &str) -> String {
    crate::hex(&crate::sha256(&[b"filescape-serial/v1\0", canonical.as_bytes()]))
}

/// The last four characters, for showing which serial a licence is without revealing it.
pub fn hint(canonical: &str) -> String {
    format!("…{}", &canonical[canonical.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;

    #[test]
    fn round_trip_and_forgiving_input() {
        let rng = SystemRandom::new();
        for kind in [LicenceKind::Temporary, LicenceKind::Perpetual] {
            for _ in 0..200 {
                let s = generate(kind, &rng);
                assert_eq!(s.len(), 3 + 4 * 6);
                assert_eq!(normalize(&s), Ok((kind, s.clone())));
                let messy = format!("  {} ", s.to_lowercase().replace('-', " ").replace('0', "o").replace('1', "l"));
                assert_eq!(normalize(&messy), Ok((kind, s.clone())));
            }
        }
    }

    #[test]
    fn single_typos_are_caught() {
        let rng = SystemRandom::new();
        let s = generate(LicenceKind::Temporary, &rng);
        let (_, canon) = normalize(&s).unwrap();
        let bytes = canon.as_bytes();
        let mut caught = 0;
        let mut tried = 0;
        for i in 4..bytes.len() {
            if bytes[i] == b'-' {
                continue;
            }
            for &c in ALPHABET.iter().filter(|&&c| c != bytes[i]) {
                let mut b = bytes.to_vec();
                b[i] = c;
                tried += 1;
                if normalize(std::str::from_utf8(&b).unwrap()) == Err(SerialError::Typo) {
                    caught += 1;
                }
            }
        }
        // 20 check bits: about one change in a million slips through.
        assert!(caught >= tried - 1, "{caught} of {tried}");
    }

    #[test]
    fn rejects_wrong_shapes() {
        assert_eq!(normalize("hello"), Err(SerialError::Malformed));
        assert_eq!(normalize("DMT-00000-00000-00000-00000"), Err(SerialError::Malformed));
        assert_eq!(normalize("FST-00000-00000-00000-0000U"), Err(SerialError::Malformed));
    }
}
