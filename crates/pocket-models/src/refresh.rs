use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::catalog::Catalog;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RefreshError {
    #[error("catalog is unsigned or signature does not verify")]
    BadSignature,
    #[error("signature is not valid base64")]
    InvalidSignatureEncoding,
    #[error("catalog JSON did not parse")]
    InvalidJson,
    #[error("catalog_version {new} is not newer than the current {current}")]
    NotNewer { new: u32, current: u32 },
}

/// CORE-13: verifies a user-initiated catalog refresh
/// (`GET .../catalog/v1.json` + `v1.json.sig`) against the compiled-in
/// public key before accepting it, and rejects anything that isn't a
/// strictly newer `catalog_version`.
pub fn verify_and_parse(
    catalog_json: &[u8],
    signature_b64: &str,
    public_key: &VerifyingKey,
    current_catalog_version: u32,
) -> Result<Catalog, RefreshError> {
    let sig_bytes = STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| RefreshError::InvalidSignatureEncoding)?;
    let sig_array: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| RefreshError::InvalidSignatureEncoding)?;
    let signature = Signature::from_bytes(&sig_array);

    public_key
        .verify(catalog_json, &signature)
        .map_err(|_| RefreshError::BadSignature)?;

    let catalog: Catalog =
        serde_json::from_slice(catalog_json).map_err(|_| RefreshError::InvalidJson)?;

    if catalog.catalog_version <= current_catalog_version {
        return Err(RefreshError::NotNewer {
            new: catalog.catalog_version,
            current: current_catalog_version,
        });
    }

    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[3u8; 32])
    }

    fn sample_catalog_json(version: u32) -> Vec<u8> {
        serde_json::json!({
            "catalog_version": version,
            "generated_at": "2026-10-14T00:00:00Z",
            "entries": []
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn good_signature_and_newer_version_verifies() {
        let key = test_signing_key();
        let json = sample_catalog_json(2);
        let sig = STANDARD.encode(key.sign(&json).to_bytes());

        let catalog = verify_and_parse(&json, &sig, &key.verifying_key(), 1).unwrap();
        assert_eq!(catalog.catalog_version, 2);
    }

    #[test]
    fn bad_signature_is_rejected() {
        let key = test_signing_key();
        let other_key = SigningKey::from_bytes(&[5u8; 32]);
        let json = sample_catalog_json(2);
        let sig = STANDARD.encode(other_key.sign(&json).to_bytes());

        assert_eq!(
            verify_and_parse(&json, &sig, &key.verifying_key(), 1).unwrap_err(),
            RefreshError::BadSignature
        );
    }

    #[test]
    fn tampered_json_fails_signature_check() {
        let key = test_signing_key();
        let json = sample_catalog_json(2);
        let sig = STANDARD.encode(key.sign(&json).to_bytes());

        let mut tampered = json.clone();
        *tampered.last_mut().unwrap() ^= 0x01;

        assert_eq!(
            verify_and_parse(&tampered, &sig, &key.verifying_key(), 1).unwrap_err(),
            RefreshError::BadSignature
        );
    }

    #[test]
    fn rejects_non_newer_catalog_version() {
        let key = test_signing_key();
        let json = sample_catalog_json(1);
        let sig = STANDARD.encode(key.sign(&json).to_bytes());

        assert_eq!(
            verify_and_parse(&json, &sig, &key.verifying_key(), 1).unwrap_err(),
            RefreshError::NotNewer { new: 1, current: 1 }
        );
    }

    #[test]
    fn missing_or_garbage_signature_is_rejected() {
        let key = test_signing_key();
        let json = sample_catalog_json(2);
        assert_eq!(
            verify_and_parse(&json, "not-base64!!", &key.verifying_key(), 1).unwrap_err(),
            RefreshError::InvalidSignatureEncoding
        );
    }
}
