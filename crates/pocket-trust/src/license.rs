use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::license_pubkey::LicenseIssuer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseAccount {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseProduct {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicensePolicy {
    pub id: String,
    #[serde(default)]
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseData {
    pub id: String,
    pub created: String,
    #[serde(default)]
    pub expiry: Option<String>,
}

/// Signed payload shape (our Worker and legacy Keygen `ED25519_SIGN`;
/// unknown fields such as `limits` are ignored):
/// `{"account":{"id":"..."},"product":{"id":"..."},"policy":{"id":"...","duration":null},"user":null,"license":{"id":"...","created":"...","expiry":"..."}}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicensePayload {
    pub account: LicenseAccount,
    pub product: LicenseProduct,
    pub policy: LicensePolicy,
    pub license: LicenseData,
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

/// Verifies a licence key fully offline (LIC-02) against the trusted issuers.
///
/// Wire format: `key/<base64 payload>.<base64 signature>`, where the
/// signature is Ed25519 over the UTF-8 bytes of `key/<base64 payload>`
/// (everything before the final `.`). The key must be signed by one of
/// `issuers` and carry that issuer's account, product and policy IDs.
pub fn verify_license_key(
    key: &str,
    issuers: &[LicenseIssuer],
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

    let signature = decode_signature(sig_b64)?;
    let issuer = signing_issuer(issuers, message.as_bytes(), &signature)
        .ok_or(LicenseError::BadSignature)?;

    let payload_json = decode_b64(payload_b64)?;
    let payload: LicensePayload =
        serde_json::from_slice(&payload_json).map_err(|_| LicenseError::InvalidPayload)?;

    if payload.account.id != issuer.account_id {
        return Err(LicenseError::WrongAccount(payload.account.id));
    }
    if payload.product.id != issuer.product_id {
        return Err(LicenseError::WrongProduct(payload.product.id));
    }
    if payload.policy.id != issuer.policy_id {
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

fn decode_signature(sig_b64: &str) -> Result<Signature, LicenseError> {
    let sig_array: [u8; 64] = decode_b64(sig_b64)?
        .as_slice()
        .try_into()
        .map_err(|_| LicenseError::InvalidFormat)?;
    Ok(Signature::from_bytes(&sig_array))
}

fn signing_issuer<'a>(
    issuers: &'a [LicenseIssuer],
    message: &[u8],
    signature: &Signature,
) -> Option<&'a LicenseIssuer> {
    issuers.iter().find(|i| {
        VerifyingKey::from_bytes(&i.public_key)
            .is_ok_and(|vk| vk.verify(message, signature).is_ok())
    })
}

/// Signed `revoked.json` from the licence Worker:
/// `{"ids":[...],"issued":"<ISO UTC>","sig":"<b64url>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokedList {
    pub ids: Vec<String>,
    pub issued: String,
    pub sig: String,
}

/// Parses `revoked.json` and checks `sig` (Ed25519 over
/// `revoked/<issued>/<ids joined with ",">`) against the trusted issuers.
pub fn verify_revoked_list(
    json: &[u8],
    issuers: &[LicenseIssuer],
) -> Result<RevokedList, LicenseError> {
    let list: RevokedList =
        serde_json::from_slice(json).map_err(|_| LicenseError::InvalidPayload)?;
    parse_iso8601_utc_ms(&list.issued).ok_or(LicenseError::InvalidPayload)?;
    let message = format!("revoked/{}/{}", list.issued, list.ids.join(","));
    let signature = decode_signature(&list.sig)?;
    signing_issuer(issuers, message.as_bytes(), &signature).ok_or(LicenseError::BadSignature)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::license_pubkey::{KEYGEN_LEGACY, OURS_TEST, TRUSTED_ISSUERS};
    use ed25519_dalek::{Signer, SigningKey};

    const REAL_SAMPLE_KEY: &str = "key/eyJhY2NvdW50Ijp7ImlkIjoiNDBmZTcyNjAtZDc5Zi00NmFkLTljMGMtOTcyMzMxZjRmZmMwIn0sInByb2R1Y3QiOnsiaWQiOiIwYjc5MDQ1YS1kOTlkLTQwMjMtOWI4Ni02NjAwODA3YTliOGUifSwicG9saWN5Ijp7ImlkIjoiNDVhMGZmMGUtNzYzNy00M2FjLWIwNjMtODEzOTZlZDQ3MGU4IiwiZHVyYXRpb24iOm51bGx9LCJ1c2VyIjpudWxsLCJsaWNlbnNlIjp7ImlkIjoiMjNkNGFiNWItZTQzYy00YjAyLWE4ZWItYWRjNDcwNjgzOWY2IiwiY3JlYXRlZCI6IjIwMjYtMTAtMTVUMDY6NTg6MzcuMjAzWiIsImV4cGlyeSI6IjIwMjgtMDMtMzFUMTg6MzA6MDAuMDAwWiJ9fQ==.fhC7LYG5-OOt4G4mG34XETvcNXnRYNuMvx9kg-lktpuq9N7NACJndb1FVPYBYCRq-0CU-N8-eOi44w4gkkyLBA==";

    /// Minted by the licence Worker's `/issue` (test signer, product
    /// pocketai-mac, 1 day): created 2026-11-04T13:40:38.960Z, expires
    /// 2026-11-05T13:40:38.960Z.
    const WORKER_KEY: &str = "key/eyJhY2NvdW50Ijp7ImlkIjoicG9ja2V0YWkifSwicHJvZHVjdCI6eyJpZCI6InBvY2tldGFpLW1hYyJ9LCJwb2xpY3kiOnsiaWQiOiJwcm8iLCJkdXJhdGlvbiI6bnVsbH0sImxpY2Vuc2UiOnsiaWQiOiJlNDM0YTU2Ny02MmVhLTQ1ZWYtOTIyOC01YmVkOWEzZjJkMDMiLCJjcmVhdGVkIjoiMjAyNi0xMS0wNFQxMzo0MDozOC45NjBaIiwiZXhwaXJ5IjoiMjAyNi0xMS0wNVQxMzo0MDozOC45NjBaIn0sImxpbWl0cyI6eyJkZXZpY2VzIjo1fX0.71hb0-a1lb65mL_7sjL351siIW_eppJ78FAzUxBiIjiKeQs1cUwrnWfNdf-WDjTn-F9LO7y5nwlzxh6n0RXSDw";
    const WORKER_KEY_ID: &str = "e434a567-62ea-45ef-9228-5bed9a3f2d03";
    const WORKER_KEY_NOW: i64 = 1_793_799_800_000; // 2026-11-04T13:43:20Z
    const WORKER_KEY_AFTER: i64 = 1_793_923_200_000; // 2026-11-06T00:00:00Z

    /// Live `GET /revoked.json` from the test Worker.
    const WORKER_REVOKED: &str = r#"{"ids":[],"issued":"2026-11-04T13:40:39.725Z","sig":"fFFEJ_mCP-2hcNVmNrPkGR4Mqo9SEf1KupY_IQV97R3BxGOWrViavrsEYfB4FhBfgCO6yzSKc9AMeGzo31pBBA"}"#;

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

    /// A throwaway issuer with the legacy IDs, so `sample_payload` matches it.
    fn throwaway_issuer(sk: &SigningKey) -> LicenseIssuer {
        LicenseIssuer {
            name: "throwaway",
            public_key: sk.verifying_key().to_bytes(),
            ..KEYGEN_LEGACY
        }
    }

    fn sample_payload() -> LicensePayload {
        LicensePayload {
            account: LicenseAccount {
                id: KEYGEN_LEGACY.account_id.to_string(),
            },
            product: LicenseProduct {
                id: KEYGEN_LEGACY.product_id.to_string(),
            },
            policy: LicensePolicy {
                id: KEYGEN_LEGACY.policy_id.to_string(),
                duration: None,
            },
            license: LicenseData {
                id: "23d4ab5b-e43c-4b02-a8eb-adc4706839f6".to_string(),
                created: "2026-10-15T06:58:37.203Z".to_string(),
                expiry: Some("2028-03-31T18:30:00.000Z".to_string()),
            },
        }
    }

    #[test]
    fn worker_key_verifies() {
        let p = verify_license_key(WORKER_KEY, TRUSTED_ISSUERS, WORKER_KEY_NOW, &[]).unwrap();
        assert_eq!(p.license.id, WORKER_KEY_ID);
        assert_eq!(p.account.id, OURS_TEST.account_id);
        assert_eq!(p.product.id, OURS_TEST.product_id);
        assert_eq!(
            p.license.expiry.as_deref(),
            Some("2026-11-05T13:40:38.960Z")
        );
    }

    /// From the test Worker's `POST /free` (no auth, 90 days):
    /// expires 2027-02-02T13:51:27.811Z.
    const FREE_KEY: &str = "key/eyJhY2NvdW50Ijp7ImlkIjoicG9ja2V0YWkifSwicHJvZHVjdCI6eyJpZCI6InBvY2tldGFpLW1hYyJ9LCJwb2xpY3kiOnsiaWQiOiJwcm8iLCJkdXJhdGlvbiI6bnVsbH0sImxpY2Vuc2UiOnsiaWQiOiIwZDQyZGIyNC1mNGQ4LTRiNDQtODY5NS02NmFjMzE4ZTBmNDAiLCJjcmVhdGVkIjoiMjAyNi0xMS0wNFQxMzo1MToyNy44MTFaIiwiZXhwaXJ5IjoiMjAyNy0wMi0wMlQxMzo1MToyNy44MTFaIn0sImxpbWl0cyI6eyJkZXZpY2VzIjo1fX0.z-xaV7W0FI1DXfG3AbdHRWU5U6MAFFPrm8qHj7HVNA0GtherzkUayj7yLM6c8cY_8sjOYlI9-zBSAfCKARNmAQ";

    #[test]
    fn free_test_key_verifies_with_test_issuer() {
        let p = verify_license_key(FREE_KEY, &[OURS_TEST], WORKER_KEY_NOW, &[]).unwrap();
        assert_eq!(p.license.id, "0d42db24-f4d8-4b44-8695-66ac318e0f40");
        assert_eq!(
            p.license.expiry.as_deref(),
            Some("2027-02-02T13:51:27.811Z")
        );
    }

    #[test]
    fn worker_key_expires() {
        assert_eq!(
            verify_license_key(WORKER_KEY, TRUSTED_ISSUERS, WORKER_KEY_AFTER, &[]),
            Err(LicenseError::Expired)
        );
    }

    #[test]
    fn worker_key_tampered_fails() {
        let (message, sig) = WORKER_KEY.rsplit_once('.').unwrap();
        let mut bytes = message.as_bytes().to_vec();
        bytes[10] ^= 0x01;
        let tampered = format!("{}.{}", String::from_utf8(bytes).unwrap(), sig);
        assert_eq!(
            verify_license_key(&tampered, TRUSTED_ISSUERS, WORKER_KEY_NOW, &[]),
            Err(LicenseError::BadSignature)
        );
        // Our key is not accepted under the Keygen issuer alone, and vice versa.
        assert_eq!(
            verify_license_key(WORKER_KEY, &[KEYGEN_LEGACY], WORKER_KEY_NOW, &[]),
            Err(LicenseError::BadSignature)
        );
        assert_eq!(
            verify_license_key(REAL_SAMPLE_KEY, &[OURS_TEST], TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn worker_key_revoked_fails() {
        let revoked = vec![WORKER_KEY_ID.to_string()];
        assert_eq!(
            verify_license_key(WORKER_KEY, TRUSTED_ISSUERS, WORKER_KEY_NOW, &revoked),
            Err(LicenseError::Revoked(WORKER_KEY_ID.into()))
        );
    }

    #[test]
    fn worker_revoked_json_verifies() {
        let list = verify_revoked_list(WORKER_REVOKED.as_bytes(), TRUSTED_ISSUERS).unwrap();
        assert!(list.ids.is_empty());
        assert_eq!(list.issued, "2026-11-04T13:40:39.725Z");
    }

    #[test]
    fn revoked_json_tampered_fails() {
        let added = WORKER_REVOKED.replace(r#""ids":[]"#, r#""ids":["x"]"#);
        assert_eq!(
            verify_revoked_list(added.as_bytes(), TRUSTED_ISSUERS),
            Err(LicenseError::BadSignature)
        );
        let redated = WORKER_REVOKED.replace("13:40:39.725Z", "13:40:39.726Z");
        assert_eq!(
            verify_revoked_list(redated.as_bytes(), TRUSTED_ISSUERS),
            Err(LicenseError::BadSignature)
        );
        assert_eq!(
            verify_revoked_list(b"not json", TRUSTED_ISSUERS),
            Err(LicenseError::InvalidPayload)
        );
    }

    #[test]
    fn revoked_json_with_ids_signed_like_the_worker() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let issued = "2026-11-04T00:00:00.000Z";
        let msg = format!("revoked/{issued}/a,b");
        let sig = URL_SAFE_NO_PAD.encode(sk.sign(msg.as_bytes()).to_bytes());
        let json = format!(r#"{{"ids":["a","b"],"issued":"{issued}","sig":"{sig}"}}"#);
        let list = verify_revoked_list(json.as_bytes(), &[throwaway_issuer(&sk)]).unwrap();
        assert_eq!(list.ids, vec!["a", "b"]);
        assert_eq!(
            verify_revoked_list(json.as_bytes(), TRUSTED_ISSUERS),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn real_sample_key_verifies_before_expiry() {
        let payload =
            verify_license_key(REAL_SAMPLE_KEY, TRUSTED_ISSUERS, TIME_BEFORE_EXPIRY, &[]).unwrap();
        assert_eq!(payload.license.id, "23d4ab5b-e43c-4b02-a8eb-adc4706839f6");
        assert_eq!(payload.account.id, KEYGEN_LEGACY.account_id);
        assert_eq!(payload.product.id, KEYGEN_LEGACY.product_id);
        assert_eq!(payload.policy.id, KEYGEN_LEGACY.policy_id);
        assert_eq!(
            payload.license.expiry.as_deref(),
            Some("2028-03-31T18:30:00.000Z")
        );
    }

    #[test]
    fn real_sample_key_after_expiry_fails() {
        let err = verify_license_key(REAL_SAMPLE_KEY, TRUSTED_ISSUERS, TIME_AFTER_EXPIRY, &[])
            .unwrap_err();
        assert_eq!(err, LicenseError::Expired);
    }

    #[test]
    fn flipped_byte_in_payload_fails() {
        let (message, sig) = REAL_SAMPLE_KEY.rsplit_once('.').unwrap();
        let mut bytes = message.as_bytes().to_vec();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let tampered = format!("{}.{}", String::from_utf8(bytes).unwrap(), sig);
        assert_eq!(
            verify_license_key(&tampered, TRUSTED_ISSUERS, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn flipped_byte_in_signature_fails() {
        let (message, sig) = REAL_SAMPLE_KEY.rsplit_once('.').unwrap();
        let mut sig_bytes = URL_SAFE.decode(sig).unwrap();
        sig_bytes[0] ^= 0x01;
        let tampered = format!("{message}.{}", URL_SAFE.encode(sig_bytes));
        assert_eq!(
            verify_license_key(&tampered, TRUSTED_ISSUERS, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn wrong_public_key_fails() {
        let other_signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let other = throwaway_issuer(&other_signing_key);
        assert_eq!(
            verify_license_key(REAL_SAMPLE_KEY, &[other], TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::BadSignature)
        );
    }

    #[test]
    fn revoked_license_id_fails() {
        let revoked = vec!["23d4ab5b-e43c-4b02-a8eb-adc4706839f6".to_string()];
        assert_eq!(
            verify_license_key(
                REAL_SAMPLE_KEY,
                TRUSTED_ISSUERS,
                TIME_BEFORE_EXPIRY,
                &revoked
            ),
            Err(LicenseError::Revoked(
                "23d4ab5b-e43c-4b02-a8eb-adc4706839f6".into()
            ))
        );
    }

    #[test]
    fn wrong_account_fails() {
        let throwaway_sk = SigningKey::from_bytes(&[42u8; 32]);
        let throwaway = [throwaway_issuer(&throwaway_sk)];
        let mut payload = sample_payload();
        payload.account.id = "wrong-account-id".to_string();
        let key = make_test_key(&throwaway_sk, &payload);
        assert_eq!(
            verify_license_key(&key, &throwaway, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::WrongAccount("wrong-account-id".to_string()))
        );
    }

    #[test]
    fn wrong_product_fails() {
        let throwaway_sk = SigningKey::from_bytes(&[42u8; 32]);
        let throwaway = [throwaway_issuer(&throwaway_sk)];
        let mut payload = sample_payload();
        payload.product.id = "wrong-product-id".to_string();
        let key = make_test_key(&throwaway_sk, &payload);
        assert_eq!(
            verify_license_key(&key, &throwaway, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::WrongProduct("wrong-product-id".to_string()))
        );
    }

    #[test]
    fn wrong_policy_fails() {
        let throwaway_sk = SigningKey::from_bytes(&[42u8; 32]);
        let throwaway = [throwaway_issuer(&throwaway_sk)];
        let mut payload = sample_payload();
        payload.policy.id = "wrong-policy-id".to_string();
        let key = make_test_key(&throwaway_sk, &payload);
        assert_eq!(
            verify_license_key(&key, &throwaway, TIME_BEFORE_EXPIRY, &[]),
            Err(LicenseError::WrongPolicy("wrong-policy-id".to_string()))
        );
    }

    #[test]
    fn key_with_surrounding_whitespace_still_verifies() {
        let key = format!("  \n\t{}\t \n", REAL_SAMPLE_KEY);
        let res = verify_license_key(&key, TRUSTED_ISSUERS, TIME_BEFORE_EXPIRY, &[]);
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
