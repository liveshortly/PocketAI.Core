#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum PocketError {
    #[error("not found: {what}")]
    NotFound { what: String },
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
    #[error("will not fit: needs {needed_bytes}, available {available_bytes}: {advice}")]
    WillNotFit {
        needed_bytes: u64,
        available_bytes: u64,
        advice: String,
    },
    #[error("engine failed ({engine:?}): {message}")]
    EngineFailed {
        engine: super::EngineKind,
        message: String,
    },
    #[error("cancelled")]
    Cancelled,
    #[error("unauthorized")]
    Unauthorized,
    #[error("not entitled: {feature}")]
    NotEntitled { feature: String },
    #[error("network: {message}")]
    Network { message: String },
    #[error("storage: {message}")]
    Storage { message: String },
    #[error("internal: {message}")]
    Internal { message: String },
    #[error("callback: {message}")]
    Callback { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for PocketError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        PocketError::Callback { message: e.reason }
    }
}

pub fn unimplemented(name: &str) -> PocketError {
    PocketError::Internal {
        message: format!("unimplemented: {name}"),
    }
}
