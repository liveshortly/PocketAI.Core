uniffi::setup_scaffolding!("pocket_ffi");

mod chat_routes;
mod control_routes;
mod convert;
mod core;
#[cfg(test)]
mod core_tests;
pub mod engine_adapter;
mod error;
mod media_routes;
mod mock_stream;
mod pairing_routes;
mod traits;
mod types;

pub use core::PocketCore;
pub use engine_adapter::PocketEngineAdapter;
pub use error::PocketError;
pub use mock_stream::{mock_generate, GenerateSink};
pub use traits::*;
pub use types::*;

#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

#[uniffi::export]
pub fn sample_device_profile() -> DeviceProfile {
    DeviceProfile {
        platform: Platform::MacOs,
        model_identifier: "Mac16,10".into(),
        device_name: String::new(),
        chip: "M4".into(),
        total_ram_bytes: 16 * 1024 * 1024 * 1024,
        app_memory_limit_bytes: 8 * 1024 * 1024 * 1024,
        gpu_family: "metal3".into(),
        free_disk_bytes: 100 * 1024 * 1024 * 1024,
        thermal_state: ThermalState::Nominal,
        low_power_mode: false,
        has_increased_memory_entitlement: true,
    }
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

/// CORE-15: reasoning capability for any chat model id, including
/// "mac:<peer>/<id>" models the phone's catalog doesn't carry. Curated ids
/// return the catalog flag; anything else falls back to the family heuristic.
#[uniffi::export]
pub fn model_supports_reasoning(model_id: String) -> bool {
    let id = match model_id.strip_prefix("mac:") {
        Some(rest) => rest.split_once('/').map_or(rest, |(_, local)| local),
        None => &model_id,
    };
    pocket_models::load()
        .entries
        .iter()
        .find(|m| m.id == id)
        .map_or_else(|| pocket_models::supports_reasoning(id), |m| m.reasoning)
}
