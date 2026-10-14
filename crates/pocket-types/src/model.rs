use serde::{Deserialize, Serialize};

use crate::{ids::ModelId, Modality};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelFormat {
    MlxSafetensors,
    Gguf,
    CoreMlPackage,
    WhisperKit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LicenseInfo {
    pub spdx: String,
    pub commercial_ok: bool,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub id: ModelId,
    pub display_name: String,
    pub family: String,
    pub params_b: f32,
    pub quant: String,
    pub format: ModelFormat,
    pub modalities: Vec<Modality>,
    pub context_length: u32,
    pub files: Vec<ModelFile>,
    pub license: LicenseInfo,
    pub hf_repo: String,
    pub hf_revision: String,
    pub min_ram_bytes: u64,
    pub est_runtime_bytes: u64,
    pub tags: Vec<String>,
    pub curated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitVerdict {
    RunsWell,
    RunsSlow,
    WillNotFit,
    NotSupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Estimated,
    Measured,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Benchmark {
    pub tps: f32,
    pub ttft_ms: u32,
    pub peak_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitReport {
    pub verdict: FitVerdict,
    pub reason: String,
    pub est_tokens_per_sec: Option<f32>,
    pub measured_tokens_per_sec: Option<f32>,
    pub est_runtime_bytes: u64,
    pub disk_bytes: u64,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogFilter {
    pub modality: Option<Modality>,
    pub curated_only: bool,
    pub installed_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelState {
    Available,
    Queued,
    Downloading,
    Paused,
    Verifying,
    Installed,
    Failed { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub manifest: ModelManifest,
    pub fit: FitReport,
    pub state: ModelState,
    pub bytes_done: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadedModelInfo {
    pub handle: String,
    pub id: ModelId,
    pub engine: crate::EngineKind,
    pub resident_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadOptions {
    pub context_length: Option<u32>,
    pub pinned_engine: Option<crate::EngineKind>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadedModel {
    pub handle: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DownloadPlan {
    pub files: Vec<PlannedFile>,
    pub total_bytes: u64,
    pub dest_dir: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedFile {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub dest_part: String,
    pub dest_final: String,
}
