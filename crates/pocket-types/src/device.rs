use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Ios,
    MacOs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modality {
    Text,
    Vision,
    Embedding,
    SpeechToText,
    TextToSpeech,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineKind {
    Mlx,
    LlamaCpp,
    CoreMl,
    WhisperKit,
    AppleSpeech,
    AppleFoundation,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
