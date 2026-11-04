mod http;
mod license;
mod license_pubkey;
mod pairing;
mod tls;

pub use http::{OfflinePauseHook, PocketHttp};
pub use license::{
    parse_iso8601_utc_ms, verify_license_key, verify_revoked_list, LicenseError, LicensePayload,
    RevokedList,
};
pub use license_pubkey::{
    LicenseIssuer, KEYGEN_LEGACY, LICENSE_SERVER_BASE_URL, OURS_TEST, PRO_CHECKOUT_URL,
    TRUSTED_ISSUERS,
};
pub use pairing::{
    build_pairing_uri, generate_manual_code, generate_secret, normalize_manual_code, pairing_sas,
    parse_pairing_uri, redeem_manual_code, ManualCode, ManualCodeError, PairingUri,
    PairingUriError, MANUAL_CODE_MAX_ATTEMPTS,
};
pub use tls::{der_to_pem, generate_identity, identity_from_der, TlsIdentity};
