use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::keygen_pubkey::{ACCOUNT_ID, PRODUCT_ID, PRO_POLICY_ID};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeygenAccount {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeygenProduct {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeygenPolicy {
    pub id: String,
    #[serde(default)]
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeygenLicenseData {
    pub id: String,
    pub created: String,
    #[serde(default)]
    pub expiry: Option<String>,
}

/// Keygen `ED25519_SIGN` payload shape:
/// `{"account":{"id":"..."},"product":{"id":"..."},"policy":{"id":"...","duration":null},"user":null,"license":{"id":"...","created":"...","expiry":"..."}}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicensePayload {
    pub account: KeygenAccount,
    pub product: KeygenProduct,
    pub policy: KeygenPolicy,
    pub license: KeygenLicenseData,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LicenseError {
    #[error("invalid key format")]
    InvalidFormat,
    #[error("signature verification failed")]
    BadSignature,
    #[error("payload is not valid JSON")]
    InvalidPayload,
    #[error("license account '{0}' does not match expected account")]
    WrongAccount(String),
    #[error("license product '{0}' does not match expected product")]
    WrongProduct(String),
    #[error("license policy '{0}' does not match expected policy")]
    WrongPolicy(String),
    #[error("license has expired")]
    Expired,
    #[error("license {0} has been revoked")]
    Revoked(String),
}

/// Parses an ISO 8601 / RFC 3339 UTC timestamp (e.g. `2028-03-31T18:30:00.000Z` or `2028-03-31T18:30:00Z`)
/// into milliseconds since Unix epoch (1970-01-01T00:00:00Z).
pub fn parse_iso8601_utc_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = s.strip_suffix('Z').or_else(|| s.strip_suffix('z'))?;
    let (date_part, time_part) = s.split_once(['T', 't'])?;

    let mut date_iter = date_part.split('-');
    let year: i64 = date_iter.next()?.parse().ok()?;
    let month: i64 = date_iter.next()?.parse().ok()?;
    let day: i64 = date_iter.next()?.parse().ok()?;
    if date_iter.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let (time_main, ms) = if let Some((tm, frac)) = time_part.split_once('.') {
        if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let frac_padded = format!("{:0<3}", frac);
        let ms_str = &frac_padded[..3];
        let ms: i64 = ms_str.parse().ok()?;
        (tm, ms)
    } else {
        (time_part, 0)
    };

    let mut time_iter = time_main.split(':');
    let hour: i64 = time_iter.next()?.parse().ok()?;
    let min: i64 = time_iter.next()?.parse().ok()?;
    let sec: i64 = time_iter.next()?.parse().ok()?;
    if time_iter.next().is_some()
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min)
        || !(0..=60).contains(&sec)
    {
        return None;
    }

    // Howard Hinnant's algorithm for days since civil epoch (1970-01-01)
    let mut y = year;
    if month <= 2 {
        y -= 1;
    }
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let doy = (153 * (if month <= 2 { month + 9 } else { month - 3 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    let total_sec = days * 86400 + hour * 3600 + min * 60 + sec;
    Some(total_sec * 1000 + ms)
}

fn decode_b64(input: &str) -> Result<Vec<u8>, LicenseError> {
    URL_SAFE
        .decode(input)
        .or_else(|_| URL_SAFE_NO_PAD.decode(input))
        .or_else(|_| STANDARD.decode(input))
        .or_else(|_| STANDARD_NO_PAD.decode(input))
        .map_err(|_| LicenseError::InvalidFormat)
}

/// Verifies a Keygen `ED25519_SIGN` license key fully offline (LIC-02).
///
/// Wire format: `key/<base64 payload>.<base64 signature>`, where the
/// signature is Ed25519 over the UTF-8 bytes of `key/<base64 payload>`
/// (everything before the final `.`).
pub fn verify_license_key(
    key: &str,
    public_key: &VerifyingKey,
    now_unix_ms: i64,
    revoked_licenses: &[String],
) -> Result<LicensePayload, LicenseError> {
    let trimmed = key.trim();
    let (message, sig_b64) = trimmed
        .rsplit_once('.')
        .ok_or(LicenseError::InvalidFormat)?;

    let payload_b64 = message
        .strip_prefix("key/")
        .ok_or(LicenseError::InvalidFormat)?;

    let sig_bytes = decode_b64(sig_b64)?;
    let sig_array: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| LicenseError::InvalidFormat)?;
    let signature = Signature::from_bytes(&sig_array);

    public_key
        .verify(message.as_bytes(), &signature)
        .map_err(|_| LicenseError::BadSignature)?;

    let payload_json = decode_b64(payload_b64)?;
    let payload: LicensePayload =
        serde_json::from_slice(&payload_json).map_err(|_| LicenseError::InvalidPayload)?;

    if payload.account.id != ACCOUNT_ID {
        return Err(LicenseError::WrongAccount(payload.account.id));
    }
    if payload.product.id != PRODUCT_ID {
        return Err(LicenseError::WrongProduct(payload.product.id));
    }
    if payload.policy.id != PRO_POLICY_ID {
        return Err(LicenseError::WrongPolicy(payload.policy.id));
    }
    if revoked_licenses.iter().any(|id| id == &payload.license.id) {
        return Err(LicenseError::Revoked(payload.license.id));
    }
    if let Some(ref expiry_str) = payload.license.expiry {
        let expiry_ms = parse_iso8601_utc_ms(expiry_str).ok_or(LicenseError::InvalidPayload)?;
        if expiry_ms < now_unix_ms {
            return Err(LicenseError::Expired);
        }
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keygen_pubkey::keygen_verifying_key;
    use ed25519_dalek::{Signer, SigningKey};

    const REAL_SAMPLE_KEY: &str = "key/eyJhY2NvdW50Ijp7ImlkIjoiNDBmZTcyNjAtZDc5Zi00NmFkLTljMGMtOTcyMzMxZjRmZmMwIn0sInByb2R1Y3QiOnsiaWQiOiIwYjc5MDQ1YS1kOTlkLTQwMjMtOWI4Ni02NjAwODA3YTliOGUifSwicG9saWN5Ijp7ImlkIjoiNDVhMGZmMGUtNzYzNy00M2FjLWIwNjMtODEzOTZlZDQ3MGU4IiwiZHVyYXRpb24iOm51bGx9LCJ1c2VyIjpudWxsLCJsaWNlbnNlIjp7ImlkIjoiMjNkNGFiNWItZTQzYy00YjAyLWE4ZWItYWRjNDcwNjgzOWY2IiwiY3JlYXRlZCI6IjIwMjYtMTAtMTVUMDY6NTg6MzcuMjAzWiIsImV4cGlyeSI6IjIwMjgtMDMtMzFUMTg6MzA6MDAuMDAwWiJ9fQ==.fhC7LYG5-OOt4G4mG34XETvcNXnRYNuMvx9kg-lktpuq9N7NACJndb1FVPYBYCRq-0CU-N8-eOi44w4gkkyLBA==";

    // 2028-03-31T18:30:00.000Z is 1_838_140_200_000 ms
    const TIME_BEFORE_EXPIRY: i64 = 1_760_400_000_000; // ~Oct 2025
    const TIME_AFTER_EXPIRY: i64 = 1_850_000_000_000; // ~mid 2028

    fn make_test_key(signing_key: &SigningKey, payload: &LicensePayload) -> String {
        let payload_json = serde_json::to_vec(payload).unwrap();
        let payload_b64 = URL_SAFE.encode(payload_json);
        let message = format!("key/{payload_b64}");
        let signature = signing_key.sign(message.as_bytes());
        let sig_b64 = URL_SAFE.encode(signature.to_bytes());
        format!("{message}.{sig_b64}")
    }

    fn sample_payload() -> LicensePayload {
        LicensePayload {
            account: KeygenAccount {
                id: ACCOUNT_ID.to_string(),
            },
            product: KeygenProduct {
                id: PRODUCT_ID.to_string(),
            },
            policy: KeygenPolicy {
                id: PRO_POLICY_ID.to_string(),
                duration: None,
            },
            license: KeygenLicenseData {
                id: "23d4ab5b-e43c-4b02-a8eb-adc4706839f6".to_string(),
                created: "2026-10-15T06:58:37.203Z".to_string(),
                expiry: Some("2028-03-31T18:30:00.000Z".to_string()),
            },
        }
    }

    #[test]
    fn real_sample_key_verifies_before_expiry() {
        let vk = keygen_verifying_key().unwrap();
        let payload = verify_license_key(REAL_SAMPLE_KEY, &vk, TIME_BEFORE_EXPIRY, &[]).unwrap();
        assert_eq!(payload.license.id, "23d4ab5b-e43c-4b02-a8eb-adc4706839f6");
        assert_eq!(payload.account.id, ACCOUNT_ID);
        assert_eq!(payload.product.id, PRODUCT_ID);
        assert_eq!(payload.policy.id, PRO_POLICY_ID);
        assert_eq!(
            payload.license.expiry.as_deref(),
            Some("2028-03-31T18:30:00.000Z")
        );
    }

    #[test]
    fn real_sample_key_after_expiry_fails() {
        let vk = keygen_verifying_key().unwrap();
        let err = verify_license_key(REAL_SAMPLE_KEY, &vk, TIME_AFTER_EXPIRY, &[]).unwrap_err();
        assert_eq!(err, LicenseError::Expired);
    }

    #[test]
    fn flipped_byte_in_payload_fails() {
        let vk = keygen_verifying_key().unwrap();
        let (message, sig) = REAL_SAMPLE_KEY.rsplit_once('.').unwrap();
        let mut bytes = message.as_bytes().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let tampered = format!("{}.{}", String::from_utf8(bytes).unwrap(), sig);
        assert_eq!(
            verify_license_key(&tampered, &vk, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn flipped_byte_in_signature_fails() {
        let vk = keygen_verifying_key().unwrap();
        let (message, sig) = REAL_SAMPLE_KEY.rsplit_once('.').unwrap();
        let mut sig_bytes = URL_SAFE.decode(sig).unwrap();
        sig_bytes[0] ^= 0x01;
        let tampered = format!("{message}.{}", URL_SAFE.encode(sig_bytes));
        assert_eq!(
            verify_license_key(&tampered, &vk, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn wrong_public_key_fails() {
        let other_signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let other_vk = other_signing_key.verifying_key();
        assert_eq!(
            verify_license_key(REAL_SAMPLE_KEY, &other_vk, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn revoked_license_id_fails() {
        let vk = keygen_verifying_key().unwrap();
        let revoked = vec!["23d4ab5b-e43c-4b02-a8eb-adc4706839f6".to_string()];
        assert_eq!(
            verify_license_key(REAL_SAMPLE_KEY, &vk, TIME_BEFORE_EXPIRY, &revoked),
            Err(LicenseError::Revoked(
                "23d4ab5b-e43c-4b02-a8eb-adc4706839f6".into()
            ))
        );
    }

    #[test]
    fn wrong_account_fails() {
        let throwaway_sk = SigningKey::from_bytes(&[42u8; 32]);
        let throwaway_vk = throwaway_sk.verifying_key();
        let mut payload = sample_payload();
        payload.account.id = "wrong-account-id".to_string();
        let key = make_test_key(&throwaway_sk, &payload);
        assert_eq!(
            verify_license_key(&key, &throwaway_vk, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::WrongAccount("wrong-account-id".to_string()))
        );
    }

    #[test]
    fn wrong_product_fails() {
        let throwaway_sk = SigningKey::from_bytes(&[42u8; 32]);
        let throwaway_vk = throwaway_sk.verifying_key();
        let mut payload = sample_payload();
        payload.product.id = "wrong-product-id".to_string();
        let key = make_test_key(&throwaway_sk, &payload);
        assert_eq!(
            verify_license_key(&key, &throwaway_vk, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::WrongProduct("wrong-product-id".to_string()))
        );
    }

    #[test]
    fn wrong_policy_fails() {
        let throwaway_sk = SigningKey::from_bytes(&[42u8; 32]);
        let throwaway_vk = throwaway_sk.verifying_key();
        let mut payload = sample_payload();
        payload.policy.id = "wrong-policy-id".to_string();
        let key = make_test_key(&throwaway_sk, &payload);
        assert_eq!(
            verify_license_key(&key, &throwaway_vk, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::WrongPolicy("wrong-policy-id".to_string()))
        );
    }

    #[test]
    fn key_with_surrounding_whitespace_still_verifies() {
        let vk = keygen_verifying_key().unwrap();
        let key = format!("  \n\t{}\t \n", REAL_SAMPLE_KEY);
        let res = verify_license_key(&key, &vk, TIME_BEFORE_EXPIRY, &[]);
        assert!(res.is_ok());
    }

    #[test]
    fn parse_iso8601_utc_ms_various_formats() {
        assert_eq!(parse_iso8601_utc_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601_utc_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_iso8601_utc_ms("2026-10-15T06:58:37.203Z"),
            Some(1792047517203)
        );
        assert_eq!(
            parse_iso8601_utc_ms("2028-03-31T18:30:00.000Z"),
            Some(1838140200000)
        );
        assert_eq!(
            parse_iso8601_utc_ms("2028-03-31T18:30:00Z"),
            Some(1838140200000)
        );
        assert!(parse_iso8601_utc_ms("invalid").is_none());
        assert!(parse_iso8601_utc_ms("2028-03-31T18:30:00+05:00").is_none());
    }
}
