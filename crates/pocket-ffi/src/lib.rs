uniffi::setup_scaffolding!("pocket_ffi");

#[derive(uniffi::Enum)]
pub enum Platform {
    Ios,
    MacOs,
}

#[derive(uniffi::Enum)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(uniffi::Record)]
pub struct DeviceProfile {
    pub platform: Platform,
    pub model_identifier: String,
    pub chip: String,
    pub total_ram_bytes: u64,
    pub app_memory_limit_bytes: u64,
    pub gpu_family: String,
    pub free_disk_bytes: u64,
    pub thermal_state: ThermalState,
    pub low_power_mode: bool,
    pub has_increased_memory_entitlement: bool,
}

impl From<pocket_types::DeviceProfile> for DeviceProfile {
    fn from(p: pocket_types::DeviceProfile) -> Self {
        Self {
            platform: match p.platform {
                pocket_types::Platform::Ios => Platform::Ios,
                pocket_types::Platform::MacOs => Platform::MacOs,
            },
            model_identifier: p.model_identifier,
            chip: p.chip,
            total_ram_bytes: p.total_ram_bytes,
            app_memory_limit_bytes: p.app_memory_limit_bytes,
            gpu_family: p.gpu_family,
            free_disk_bytes: p.free_disk_bytes,
            thermal_state: match p.thermal_state {
                pocket_types::ThermalState::Nominal => ThermalState::Nominal,
                pocket_types::ThermalState::Fair => ThermalState::Fair,
                pocket_types::ThermalState::Serious => ThermalState::Serious,
                pocket_types::ThermalState::Critical => ThermalState::Critical,
            },
            low_power_mode: p.low_power_mode,
            has_increased_memory_entitlement: p.has_increased_memory_entitlement,
        }
    }
}

#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

#[uniffi::export]
pub fn sample_device_profile() -> DeviceProfile {
    pocket_types::DeviceProfile {
        platform: pocket_types::Platform::MacOs,
        model_identifier: "Mac16,10".into(),
        chip: "M4".into(),
        total_ram_bytes: 16 * 1024 * 1024 * 1024,
        app_memory_limit_bytes: 8 * 1024 * 1024 * 1024,
        gpu_family: "metal3".into(),
        free_disk_bytes: 100 * 1024 * 1024 * 1024,
        thermal_state: pocket_types::ThermalState::Nominal,
        low_power_mode: false,
        has_increased_memory_entitlement: true,
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
