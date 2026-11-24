//! Serials kept to re-send: AES-256-GCM under the `SERIAL_KEY` secret. A copy of the database
//! alone doesn't reveal any serial.

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::SecureRandom;

const AAD: &[u8] = b"filescape-serial-enc/v1";

pub fn seal(key: &[u8; 32], rng: &dyn SecureRandom, serial: &str) -> String {
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).expect("32-byte key"));
    let mut nonce = [0u8; 12];
    rng.fill(&mut nonce).expect("random");
    let mut buf = serial.as_bytes().to_vec();
    k.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut buf).expect("seal");
    let mut out = nonce.to_vec();
    out.extend(buf);
    data_encoding::BASE64URL_NOPAD.encode(&out)
}

pub fn open(key: &[u8; 32], sealed: &str) -> Option<String> {
    let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let raw = data_encoding::BASE64URL_NOPAD.decode(sealed.as_bytes()).ok()?;
    if raw.len() < 12 + 16 {
        return None;
    }
    let (nonce, ct) = raw.split_at(12);
    let mut buf = ct.to_vec();
    let plain = k.open_in_place(Nonce::try_assume_unique_for_key(nonce).ok()?, Aad::from(AAD), &mut buf).ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

/// A secret given as base64url (or standard base64) of 32 bytes.
pub fn key32(b64: &str) -> Option<[u8; 32]> {
    let t = b64.trim();
    let raw = data_encoding::BASE64URL_NOPAD
        .decode(t.trim_end_matches('=').replace('+', "-").replace('/', "_").as_bytes())
        .ok()?;
    raw.try_into().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let rng = ring::rand::SystemRandom::new();
        let key = [7u8; 32];
        let s = super::seal(&key, &rng, "FSP-AAAAA-BBBBB-CCCCC-DDDDD");
        assert_eq!(super::open(&key, &s).as_deref(), Some("FSP-AAAAA-BBBBB-CCCCC-DDDDD"));
        assert_eq!(super::open(&[8u8; 32], &s), None);
    }
}
