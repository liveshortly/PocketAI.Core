mod http;
mod keygen_pubkey;
mod license;
mod pairing;
mod tls;

pub use http::{OfflinePauseHook, PocketHttp};
pub use keygen_pubkey::{
    keygen_verifying_key, ACCOUNT_ID, KEYGEN_ACCOUNT_ID, KEYGEN_PRODUCT_ID, KEYGEN_PRO_POLICY_ID,
    KEYGEN_PUBLIC_KEY, PRODUCT_ID, PRO_POLICY_ID,
};
pub use license::{parse_iso8601_utc_ms, verify_license_key, LicenseError, LicensePayload};
pub use pairing::{
    build_pairing_uri, generate_manual_code, generate_secret, normalize_manual_code, pairing_sas,
    parse_pairing_uri, redeem_manual_code, ManualCode, ManualCodeError, PairingUri,
    PairingUriError, MANUAL_CODE_MAX_ATTEMPTS,
};
pub use tls::{der_to_pem, generate_identity, identity_from_der, TlsIdentity};
