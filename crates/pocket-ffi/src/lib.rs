uniffi::setup_scaffolding!("pocket_ffi");

#[derive(uniffi::Record)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub size_bytes: u64,
}

impl From<pocket_types::ModelInfo> for ModelInfo {
    fn from(m: pocket_types::ModelInfo) -> Self {
        Self {
            id: m.id,
            display_name: m.display_name,
            size_bytes: m.size_bytes,
        }
    }
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PocketError {
    #[error("not found: {0}")]
    NotFound(String),
}

#[uniffi::export]
pub fn ping() -> String {
    pocket_types::ping()
}

#[uniffi::export]
pub fn sample_model() -> ModelInfo {
    pocket_types::ModelInfo {
        id: "sample".into(),
        display_name: "Sample Model".into(),
        size_bytes: 0,
    }
    .into()
}

/// Rust-to-Swift callback: foreign side implements this and Core drives it (e.g. download progress).
#[uniffi::export(with_foreign)]
pub trait ProgressDelegate: Send + Sync {
    fn on_progress(&self, percent: u32);
}

#[uniffi::export]
pub fn drive_progress(delegate: std::sync::Arc<dyn ProgressDelegate>) {
    for pct in [0u32, 50, 100] {
        delegate.on_progress(pct);
    }
}
