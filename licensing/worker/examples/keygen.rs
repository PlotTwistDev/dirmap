//! Makes the server's secrets. Run once, on your own computer:
//!
//!     cargo run --manifest-path licensing/worker/Cargo.toml --example keygen
//!
//! It prints the signing key (a Worker secret), its public key (built into the app) and a
//! serial-encryption key (a Worker secret). Nothing is saved anywhere.

use filescape_licence::sign::Signer;
use ring::rand::{SecureRandom, SystemRandom};

fn main() {
    let rng = SystemRandom::new();
    let pkcs8 = Signer::generate_pkcs8(&rng);
    let signer = Signer::from_pkcs8(&pkcs8).unwrap();
    let mut serial_key = [0u8; 32];
    rng.fill(&mut serial_key).unwrap();
    let b64 = |b: &[u8]| data_encoding::BASE64URL_NOPAD.encode(b);
    println!("Worker secrets (wrangler secret put NAME, then paste the value):");
    println!("  SIGNING_KEY = {}", b64(&pkcs8));
    println!("  SERIAL_KEY  = {}", b64(&serial_key));
    println!();
    println!("Build the app with this public key (key ID {}):", signer.key_id());
    println!("  FILESCAPE_LICENCE_PUBKEY = {}", b64(&signer.public_key()));
    println!();
    println!("Keep SIGNING_KEY and SERIAL_KEY in a password manager. Losing SIGNING_KEY means");
    println!("issuing new app builds; losing SERIAL_KEY means stored serials can't be emailed again.");
}
