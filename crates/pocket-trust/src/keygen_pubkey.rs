use ed25519_dalek::{SignatureError, VerifyingKey};

pub const ACCOUNT_ID: &str = "40fe7260-d79f-46ad-9c0c-972331f4ffc0";
pub const PRODUCT_ID: &str = "0b79045a-d99d-4023-9b86-6600807a9b8e";
pub const PRO_POLICY_ID: &str = "45a0ff0e-7637-43ac-b063-81396ed470e8";

pub const KEYGEN_ACCOUNT_ID: &str = ACCOUNT_ID;
pub const KEYGEN_PRODUCT_ID: &str = PRODUCT_ID;
pub const KEYGEN_PRO_POLICY_ID: &str = PRO_POLICY_ID;

/// Account Ed25519 public key (32 bytes), decoded from hex
/// `5e8141873c4cc8126e056994edf94846968307dc21552c347e1382d8cc195f17`.
pub const KEYGEN_PUBLIC_KEY: [u8; 32] = [
    0x5e, 0x81, 0x41, 0x87, 0x3c, 0x4c, 0xc8, 0x12, 0x6e, 0x05, 0x69, 0x94, 0xed, 0xf9, 0x48, 0x46,
    0x96, 0x83, 0x07, 0xdc, 0x21, 0x55, 0x2c, 0x34, 0x7e, 0x13, 0x82, 0xd8, 0xcc, 0x19, 0x5f, 0x17,
];

/// Returns the Keygen account's Ed25519 verifying key.
pub fn keygen_verifying_key() -> Result<VerifyingKey, SignatureError> {
    VerifyingKey::from_bytes(&KEYGEN_PUBLIC_KEY)
}
