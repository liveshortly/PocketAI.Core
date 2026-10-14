use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    pub id: String,
    pub seq: u64,
    pub ts: i64,
    pub source: String,
    pub event_type: String,
    pub data_json: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub lan_enabled: bool,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub running: bool,
    pub lan_enabled: bool,
    pub port: u16,
    pub connected_devices: u32,
}
