//! Signed envelopes.
//!
//! `{format, purpose, key_id, payload, signature}`, where `payload` is the exact JSON bytes that
//! were signed (base64url) and the signature covers `filescape-licence/v1/<purpose>\n<payload>`.
//! Binding the purpose into the signature means an issuance certificate (proof of purchase) can
//! never be passed off as an activation certificate (which unlocks features).
//!
//! The private key exists only on the server (a Worker secret, plus Rich's offline copy). The app
//! holds a list of public keys, so the key can be rotated with a release.

use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const FORMAT: &str = "filescape-signed/1";
const B64: data_encoding::Encoding = data_encoding::BASE64URL_NOPAD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Unlocks features on one device.
    Activation,
    /// Proves a licence was issued; unlocks nothing.
    Issuance,
    /// The server's clock, for the app's clock checks.
    Time,
    /// The server's authenticated word that an activation ended or its licence was revoked
    /// ([`crate::model::StatusStatement`]). Unlocks nothing.
    Status,
    /// The newest release ([`crate::release::ReleaseManifest`]), for the app's update notice.
    /// Unlocks nothing.
    Release,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Activation => "activation",
            Purpose::Issuance => "issuance",
            Purpose::Time => "time",
            Purpose::Status => "status",
            Purpose::Release => "release",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub purpose: String,
    pub key_id: String,
    pub payload: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    Format,
    WrongPurpose,
    UnknownKey,
    BadSignature,
    BadPayload,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            VerifyError::Format => "not a Filescape signed document",
            VerifyError::WrongPurpose => "a different kind of Filescape document",
            VerifyError::UnknownKey => "signed with a key this version of Filescape doesn't know",
            VerifyError::BadSignature => "the signature doesn't match: the file was changed",
            VerifyError::BadPayload => "the signed contents can't be read",
        })
    }
}

fn message(purpose: &str, payload: &[u8]) -> Vec<u8> {
    let mut m = format!("filescape-licence/v1/{purpose}\n").into_bytes();
    m.extend_from_slice(payload);
    m
}

/// The short ID of a public key: the first 8 bytes of its SHA-256, in hex.
pub fn key_id(public_key: &[u8]) -> String {
    crate::hex(&crate::sha256(&[public_key])[..8])
}

pub struct Signer {
    pair: Ed25519KeyPair,
    key_id: String,
}

impl Signer {
    /// A new private key in PKCS#8, to be stored as a secret.
    pub fn generate_pkcs8(rng: &dyn ring::rand::SecureRandom) -> Vec<u8> {
        Ed25519KeyPair::generate_pkcs8(rng).expect("key generation failed").as_ref().to_vec()
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Signer, String> {
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|e| format!("not an Ed25519 key: {e}"))?;
        let key_id = key_id(pair.public_key().as_ref());
        Ok(Signer { pair, key_id })
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.pair.public_key().as_ref().to_vec()
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn sign<T: Serialize>(&self, purpose: Purpose, value: &T) -> Envelope {
        let payload = serde_json::to_vec(value).expect("licence documents always serialise");
        let sig = self.pair.sign(&message(purpose.as_str(), &payload));
        Envelope {
            format: FORMAT.into(),
            purpose: purpose.as_str().into(),
            key_id: self.key_id.clone(),
            payload: B64.encode(&payload),
            signature: B64.encode(sig.as_ref()),
        }
    }
}

/// The public keys this build trusts.
#[derive(Clone, Default)]
pub struct Verifier {
    keys: Vec<(String, Vec<u8>)>,
}

impl Verifier {
    pub fn new<K: AsRef<[u8]>>(public_keys: &[K]) -> Verifier {
        Verifier { keys: public_keys.iter().map(|k| (key_id(k.as_ref()), k.as_ref().to_vec())).collect() }
    }

    pub fn public_key(&self, key_id: &str) -> Option<&[u8]> {
        self.keys.iter().find(|(id, _)| id == key_id).map(|(_, k)| k.as_slice())
    }

    pub fn verify<T: DeserializeOwned>(&self, env: &Envelope, purpose: Purpose) -> Result<T, VerifyError> {
        if env.format != FORMAT {
            return Err(VerifyError::Format);
        }
        if env.purpose != purpose.as_str() {
            return Err(VerifyError::WrongPurpose);
        }
        let key = self.public_key(&env.key_id).ok_or(VerifyError::UnknownKey)?;
        let payload = B64.decode(env.payload.as_bytes()).map_err(|_| VerifyError::Format)?;
        let sig = B64.decode(env.signature.as_bytes()).map_err(|_| VerifyError::Format)?;
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&message(&env.purpose, &payload), &sig)
            .map_err(|_| VerifyError::BadSignature)?;
        serde_json::from_slice(&payload).map_err(|_| VerifyError::BadPayload)
    }
}

/// Base64url, as used in envelopes and certificate files.
pub fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}
