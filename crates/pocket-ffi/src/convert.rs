//! Converts between `pocket-models`/`pocket-store`'s plain Rust types and the
//! FFI-facing records in `crate::types` (DECISIONS.md D19 - these can't be
//! the same type across the FFI boundary).
use crate::types::*;

pub fn modality_from_str(s: &str) -> Modality {
    match s {
        "vision" => Modality::Vision,
        "embedding" => Modality::Embedding,
        "speech_to_text" => Modality::SpeechToText,
        "text_to_speech" => Modality::TextToSpeech,
        "image" => Modality::Image,
        _ => Modality::Text,
    }
}

pub fn modality_to_str(m: &Modality) -> &'static str {
    match m {
        Modality::Text => "text",
        Modality::Vision => "vision",
        Modality::Embedding => "embedding",
        Modality::SpeechToText => "speech_to_text",
        Modality::TextToSpeech => "text_to_speech",
        Modality::Image => "image",
    }
}

pub fn format_from_str(s: &str) -> ModelFormat {
    match s {
        "gguf" => ModelFormat::Gguf,
        "coreml" => ModelFormat::CoreMlPackage,
        "whisperkit" => ModelFormat::WhisperKit,
        _ => ModelFormat::MlxSafetensors,
    }
}

pub fn manifest_from_models(m: &pocket_models::ModelManifest) -> ModelManifest {
    ModelManifest {
        id: m.id.clone(),
        display_name: m.display_name.clone(),
        family: m.family.clone(),
        params_b: m.params_b.unwrap_or(0.0),
        quant: m.quant.clone(),
        format: format_from_str(&m.format),
        modalities: m.modalities.iter().map(|s| modality_from_str(s)).collect(),
        context_length: m.context_length,
        files: m
            .files
            .iter()
            .map(|f| ModelFile {
                path: f.path.clone(),
                size: f.size,
                sha256: f.sha256.clone(),
                url: f.url.clone(),
            })
            .collect(),
        license: LicenseInfo {
            spdx: m.license.spdx.clone(),
            commercial_ok: m.license.commercial_ok,
            url: m.license.url.clone(),
        },
        hf_repo: m.hf_repo.clone(),
        hf_revision: m.hf_revision.clone(),
        min_ram_bytes: m.min_ram_bytes,
        est_runtime_bytes: m.est_runtime_bytes,
        tags: m.tags.clone(),
        curated: m.curated,
    }
}

pub fn fit_report_from_models(r: &pocket_models::FitReport) -> FitReport {
    FitReport {
        verdict: match r.verdict {
            pocket_models::FitVerdict::RunsWell => FitVerdict::RunsWell,
            pocket_models::FitVerdict::RunsSlow => FitVerdict::RunsSlow,
            pocket_models::FitVerdict::WillNotFit => FitVerdict::WillNotFit,
            pocket_models::FitVerdict::NotSupported => FitVerdict::NotSupported,
        },
        reason: r.reason.clone(),
        est_tokens_per_sec: r.est_tokens_per_sec,
        measured_tokens_per_sec: r.measured_tokens_per_sec,
        est_runtime_bytes: r.est_runtime_bytes,
        disk_bytes: r.disk_bytes,
        confidence: match r.confidence {
            pocket_models::Confidence::Estimated => Confidence::Estimated,
            pocket_models::Confidence::Measured => Confidence::Measured,
        },
    }
}

pub fn model_state_from_str(s: &str) -> ModelState {
    if let Some(reason) = s.strip_prefix("failed:") {
        return ModelState::Failed {
            reason: reason.to_string(),
        };
    }
    match s {
        "queued" => ModelState::Queued,
        "downloading" => ModelState::Downloading,
        // User pause and Offline-mode pause both surface as Paused; the UI
        // distinguishes the offline reason via core.is_offline() (§8.6).
        "paused" | "paused:offline" => ModelState::Paused,
        "verifying" => ModelState::Verifying,
        "installed" => ModelState::Installed,
        _ => ModelState::Available,
    }
}

pub fn ffi_platform_to_models(p: &Platform) -> pocket_models::Platform {
    match p {
        Platform::Ios => pocket_models::Platform::Ios,
        Platform::MacOs => pocket_models::Platform::MacOs,
    }
}
