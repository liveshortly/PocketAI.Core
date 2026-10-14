#[derive(Debug, thiserror::Error)]
pub enum PocketError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub size_bytes: u64,
}

pub fn ping() -> String {
    "pong".to_string()
}
