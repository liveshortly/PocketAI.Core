//! Process-global llama.cpp backend singleton initialization.

use std::sync::OnceLock;

use llama_cpp_2::llama_backend::LlamaBackend;
use pocket_types::{EngineKind, PocketError};

static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();

/// Returns a reference to the process-global `LlamaBackend` singleton.
/// `llama-cpp-2` requires initializing the global backend exactly once per process.
pub fn get_backend() -> Result<&'static LlamaBackend, PocketError> {
    match BACKEND.get_or_init(|| LlamaBackend::init().map_err(|e| e.to_string())) {
        Ok(backend) => Ok(backend),
        Err(err) => Err(PocketError::EngineFailed {
            engine: EngineKind::LlamaCpp,
            message: format!("failed to initialize llama.cpp backend: {err}"),
        }),
    }
}
