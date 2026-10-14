use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// `{"v":1,"tier":"pro","serial":"<ulid>","issued":"YYYY-MM-DD","max_major":0}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LicensePayload {
    pub v: u32,
    pub tier: String,
    pub serial: String,
    pub issued: String,
    pub max_major: u32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LicenseError {
    #[error("invalid key format")]
    InvalidFormat,
    #[error("signature verification failed")]
    BadSignature,
    #[error("payload is not valid JSON")]
    InvalidPayload,
    #[error("license tier '{0}' is not Pro")]
    WrongTier(String),
    #[error("license serial {0} has been revoked")]
    Revoked(String),
    #[error("license max_major {max_major} is below app major version {app_major}")]
    MaxMajorExceeded { max_major: u32, app_major: u32 },
}

/// Verifies a Keygen `ED25519_SIGN` license key fully offline (LIC-02).
///
/// Wire format: `key/<base64url payload>.<base64url signature>`, where the
/// signature is Ed25519 over the UTF-8 bytes of `key/<base64url payload>`
/// (everything before the final `.`). This is this project's best-effort
/// transcription of Keygen's documented format; confirm against Keygen's
/// `example-cpp-ed25519-verification` repo once LIC-01's real account exists,
/// and update this function (not just the compiled-in public key) if the
/// wire format differs (see DECISIONS.md D26).
pub fn verify_license_key(
    key: &str,
    public_key: &VerifyingKey,
    app_major_version: u32,
    revoked_serials: &[String],
) -> Result<LicensePayload, LicenseError> {
    let trimmed = key.trim();
    let (message, sig_b64) = trimmed
        .rsplit_once('.')
        .ok_or(LicenseError::InvalidFormat)?;

    let payload_b64 = message
        .strip_prefix("key/")
        .ok_or(LicenseError::InvalidFormat)?;

    let sig_bytes = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| LicenseError::InvalidFormat)?;
    let sig_array: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| LicenseError::InvalidFormat)?;
    let signature = Signature::from_bytes(&sig_array);

    public_key
        .verify(message.as_bytes(), &signature)
        .map_err(|_| LicenseError::BadSignature)?;

    let payload_json = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| LicenseError::InvalidFormat)?;
    let payload: LicensePayload =
        serde_json::from_slice(&payload_json).map_err(|_| LicenseError::InvalidPayload)?;

    if payload.tier != "pro" {
        return Err(LicenseError::WrongTier(payload.tier));
    }
    if revoked_serials.iter().any(|s| s == &payload.serial) {
        return Err(LicenseError::Revoked(payload.serial));
    }
    if payload.max_major != 0 && app_major_version > payload.max_major {
        return Err(LicenseError::MaxMajorExceeded {
            max_major: payload.max_major,
            app_major: app_major_version,
        });
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn test_keypair() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn other_test_keypair() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn make_key(signing_key: &SigningKey, payload: &LicensePayload) -> String {
        let payload_json = serde_json::to_vec(payload).unwrap();
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json);
        let message = format!("key/{payload_b64}");
        let signature = signing_key.sign(message.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        format!("{message}.{sig_b64}")
    }

    fn valid_payload() -> LicensePayload {
        LicensePayload {
            v: 1,
            tier: "pro".into(),
            serial: "01JVALIDSERIAL".into(),
            issued: "2026-01-01".into(),
            max_major: 0,
        }
    }

    #[test]
    fn valid_key_verifies() {
        let signing_key = test_keypair();
        let key = make_key(&signing_key, &valid_payload());
        let result = verify_license_key(&key, &signing_key.verifying_key(), 1, &[]).unwrap();
        assert_eq!(result.serial, "01JVALIDSERIAL");
    }

    #[test]
    fn key_with_surrounding_whitespace_still_verifies() {
        let signing_key = test_keypair();
        let key = format!("  {}  \n", make_key(&signing_key, &valid_payload()));
        assert!(verify_license_key(&key, &signing_key.verifying_key(), 1, &[]).is_ok());
    }

    #[test]
    fn flipped_byte_in_payload_fails() {
        let signing_key = test_keypair();
        let key = make_key(&signing_key, &valid_payload());
        let (message, sig) = key.rsplit_once('.').unwrap();
        let mut bytes = message.as_bytes().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let tampered = format!("{}.{}", String::from_utf8(bytes).unwrap(), sig);
        assert_eq!(
            verify_license_key(&tampered, &signing_key.verifying_key(), 1, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn flipped_byte_in_signature_fails() {
        let signing_key = test_keypair();
        let key = make_key(&signing_key, &valid_payload());
        let (message, sig) = key.rsplit_once('.').unwrap();
        let mut sig_bytes = URL_SAFE_NO_PAD.decode(sig).unwrap();
        sig_bytes[0] ^= 0x01;
        let tampered = format!("{message}.{}", URL_SAFE_NO_PAD.encode(sig_bytes));
        assert_eq!(
            verify_license_key(&tampered, &signing_key.verifying_key(), 1, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn wrong_public_key_fails() {
        let signing_key = test_keypair();
        let other_key = other_test_keypair();
        let key = make_key(&signing_key, &valid_payload());
        assert_eq!(
            verify_license_key(&key, &other_key.verifying_key(), 1, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn revoked_serial_fails() {
        let signing_key = test_keypair();
        let key = make_key(&signing_key, &valid_payload());
        let revoked = vec!["01JVALIDSERIAL".to_string()];
        assert_eq!(
            verify_license_key(&key, &signing_key.verifying_key(), 1, &revoked),
            Err(LicenseError::Revoked("01JVALIDSERIAL".into()))
        );
    }

    #[test]
    fn wrong_tier_fails() {
        let signing_key = test_keypair();
        let mut payload = valid_payload();
        payload.tier = "free".into();
        let key = make_key(&signing_key, &payload);
        assert_eq!(
            verify_license_key(&key, &signing_key.verifying_key(), 1, &[]),
            Err(LicenseError::WrongTier("free".into()))
        );
    }

    #[test]
    fn max_major_gate_rejects_newer_app() {
        let signing_key = test_keypair();
        let mut payload = valid_payload();
        payload.max_major = 1;
        let key = make_key(&signing_key, &payload);
        assert_eq!(
            verify_license_key(&key, &signing_key.verifying_key(), 2, &[]),
            Err(LicenseError::MaxMajorExceeded {
                max_major: 1,
                app_major: 2
            })
        );
        assert!(verify_license_key(&key, &signing_key.verifying_key(), 1, &[]).is_ok());
    }

    #[test]
    fn max_major_zero_means_uncapped() {
        let signing_key = test_keypair();
        let key = make_key(&signing_key, &valid_payload());
        assert!(verify_license_key(&key, &signing_key.verifying_key(), 99, &[]).is_ok());
    }
}
