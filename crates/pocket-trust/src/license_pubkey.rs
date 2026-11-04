//! Trusted licence issuers (COM-05 v2 / COM-08). A key verifies if any
//! issuer's signature and IDs match. Production adds a third entry.

/// One signer of licence keys and `revoked.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LicenseIssuer {
    pub name: &'static str,
    /// Ed25519 public key, 32 raw bytes.
    pub public_key: [u8; 32],
    pub account_id: &'static str,
    pub product_id: &'static str,
    pub policy_id: &'static str,
}

/// Our Cloudflare Worker, test signer (`licenses-test.getpocketai.app`).
/// LiveShortly `ls-pro` keys share this key but are not PocketAI licences,
/// so they fail on product.
pub const OURS_TEST: LicenseIssuer = LicenseIssuer {
    name: "ours_test",
    // hex 1bf0ecd17dd9b195b21aa58fe2c6f5c39f0fcb4d6f6323e1d9af20adcab69a28
    public_key: [
        0x1b, 0xf0, 0xec, 0xd1, 0x7d, 0xd9, 0xb1, 0x95, 0xb2, 0x1a, 0xa5, 0x8f, 0xe2, 0xc6, 0xf5,
        0xc3, 0x9f, 0x0f, 0xcb, 0x4d, 0x6f, 0x63, 0x23, 0xe1, 0xd9, 0xaf, 0x20, 0xad, 0xca, 0xb6,
        0x9a, 0x28,
    ],
    account_id: "pocketai",
    product_id: "pocketai-mac",
    policy_id: "pro",
};

// ponytail: remove after the last Keygen test key expires (90 days from 2026-11-04)
pub const KEYGEN_LEGACY: LicenseIssuer = LicenseIssuer {
    name: "keygen_legacy",
    // hex 5e8141873c4cc8126e056994edf94846968307dc21552c347e1382d8cc195f17
    public_key: [
        0x5e, 0x81, 0x41, 0x87, 0x3c, 0x4c, 0xc8, 0x12, 0x6e, 0x05, 0x69, 0x94, 0xed, 0xf9, 0x48,
        0x46, 0x96, 0x83, 0x07, 0xdc, 0x21, 0x55, 0x2c, 0x34, 0x7e, 0x13, 0x82, 0xd8, 0xcc, 0x19,
        0x5f, 0x17,
    ],
    account_id: "40fe7260-d79f-46ad-9c0c-972331f4ffc0",
    product_id: "0b79045a-d99d-4023-9b86-6600807a9b8e",
    policy_id: "45a0ff0e-7637-43ac-b063-81396ed470e8",
};

pub const TRUSTED_ISSUERS: &[LicenseIssuer] = &[OURS_TEST, KEYGEN_LEGACY];

/// The one test/production switch for the licence server and the Dodo
/// checkout. Flipping it to `false` fails to compile until the live Dodo
/// product id is filled in below; add the production issuer at the same time.
const TEST_MODE: bool = true;

/// Where `revoked.json` (and the checkout's thank-you page) live.
pub const LICENSE_SERVER_BASE_URL: &str = if TEST_MODE {
    "https://licenses-test.getpocketai.app"
} else {
    "https://licenses.getpocketai.app"
};

/// "Buy PocketAI Pro" (Mac only, D138). Dodo test mode has its own host.
pub const PRO_CHECKOUT_URL: &str = if TEST_MODE {
    "https://test.checkout.dodopayments.com/buy/pdt_0NrDfs5TKE4xj2NW0SAaq?quantity=1&redirect_url=https://licenses-test.getpocketai.app/thanks"
} else {
    // "https://checkout.dodopayments.com/buy/<live product id>?quantity=1&redirect_url=https://licenses.getpocketai.app/thanks"
    panic!("set the live Dodo product id in PRO_CHECKOUT_URL")
};
