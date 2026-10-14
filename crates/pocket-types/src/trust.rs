use serde::{Deserialize, Serialize};

use crate::Id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairingOffer {
    pub host_id: String,
    pub host_name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub cert_fingerprint: String,
    pub secret: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovedPairing {
    pub host_id: String,
    pub cert_fingerprint: String,
    pub addr: String,
    pub port: u16,
    pub device_id: Id,
    pub token_keychain_ref: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub id: Id,
    pub name: String,
    pub cert_sha256: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub token_ref: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: Id,
    pub name: String,
    pub platform: crate::Platform,
    pub role: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
    pub last_addr: Option<String>,
    pub revoked_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entitlements {
    pub pro: bool,
    pub source: String,
    pub expires_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoreEntitlement {
    pub pro: bool,
    pub transaction_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditQuery {
    pub limit: u32,
    pub since_ts: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail_json: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EgressEntry {
    pub ts: i64,
    pub host: String,
    pub purpose: String,
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub initiated_by: String,
}
