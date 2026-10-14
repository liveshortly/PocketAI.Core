//! CORE-13: signs a catalog JSON file with an Ed25519 private key, writing
//! `<file>.sig` (base64-standard-encoded 64-byte signature) next to it.
//! Invoked by `scripts/sign-catalog.sh`, never run in CI/the app itself -
//! the signing key lives only with whoever publishes getpocketai.app/catalog.
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};

fn main() {
    let mut args = std::env::args().skip(1);
    let key_path = args
        .next()
        .expect("usage: sign-catalog <private-key-file> <catalog.json>");
    let catalog_path = args
        .next()
        .expect("usage: sign-catalog <private-key-file> <catalog.json>");

    let key_hex = std::fs::read_to_string(&key_path).expect("read private key file");
    let key_bytes: [u8; 32] = hex_decode(key_hex.trim())
        .try_into()
        .expect("private key must be 32 bytes hex");
    let signing_key = SigningKey::from_bytes(&key_bytes);

    let catalog_bytes = std::fs::read(&catalog_path).expect("read catalog file");
    let signature = signing_key.sign(&catalog_bytes);
    let sig_b64 = STANDARD.encode(signature.to_bytes());

    let sig_path = format!("{catalog_path}.sig");
    std::fs::write(&sig_path, &sig_b64).expect("write signature file");
    println!("wrote {sig_path}");
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("invalid hex"))
        .collect()
}
