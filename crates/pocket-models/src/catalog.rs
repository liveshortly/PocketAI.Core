use serde::{Deserialize, Serialize};

const CATALOG_JSON: &str = include_str!("../data/catalog.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseInfo {
    pub spdx: String,
    pub commercial_ok: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelManifest {
    pub id: String,
    pub display_name: String,
    pub family: String,
    pub params_b: Option<f32>,
    pub quant: String,
    pub format: String,
    pub modalities: Vec<String>,
    pub context_length: u32,
    pub files: Vec<ModelFile>,
    pub license: LicenseInfo,
    pub hf_repo: String,
    pub hf_revision: String,
    pub min_ram_bytes: u64,
    pub est_runtime_bytes: u64,
    pub tags: Vec<String>,
    pub curated: bool,
    /// CORE-14: whether the model emits thinking/reasoning tokens. Curated
    /// entries carry it in catalog.json; uncurated entries (HF search,
    /// remote ids) derive it via [`supports_reasoning`] at construction, so
    /// it's always populated at the API/FFI boundary. Defaults false so
    /// pre-CORE-14 stored manifests still parse.
    #[serde(default)]
    pub reasoning: bool,
}

/// CORE-14: family heuristic for ids that never crossed the curated catalog.
/// The only copy of the heuristic — Swift reads the resulting manifest flag.
/// Lowercase substring match on the model id (ids embed the HF repo name,
/// e.g. "mlx-community/Qwen3-8B-4bit@…").
pub fn supports_reasoning(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    // ponytail: naive substring heuristic — Qwen3 "*-Instruct-2507" variants
    // are the non-thinking split. Curated entries override this via the
    // manifest's `reasoning` flag; this only covers uncurated ids.
    if id.contains("instruct-2507") {
        return false;
    }
    ["qwen3", "qwq", "deepseek-r1", "gpt-oss", "smollm3"]
        .iter()
        .any(|family| id.contains(family))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub catalog_version: u32,
    pub generated_at: String,
    pub entries: Vec<ModelManifest>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("{repo}: file {path} missing sha256")]
    MissingSha256 { repo: String, path: String },
    #[error("{repo}: file {path} has zero size")]
    ZeroSize { repo: String, path: String },
    #[error("{repo}: license not set")]
    MissingLicense { repo: String },
    #[error("{repo}: revision '{revision}' is not a 40-char hex sha")]
    BadRevision { repo: String, revision: String },
    #[error("{repo}: has no files")]
    NoFiles { repo: String },
    #[error("tier '{tier}' has {count} recommended text models, expected exactly 1")]
    TierRecommendedCount { tier: String, count: usize },
}

fn is_40_char_hex(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn load() -> Catalog {
    serde_json::from_str(CATALOG_JSON).expect("bundled catalog.json must parse")
}

/// CORE-06's "Done when": >=20 entries, every entry's files all have
/// sha256+size, license set, and a 40-char-hex revision; exactly one
/// `recommended` text model per `tier:*` tag.
pub fn validate(catalog: &Catalog) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    for entry in &catalog.entries {
        if entry.files.is_empty() {
            errors.push(ValidationError::NoFiles {
                repo: entry.hf_repo.clone(),
            });
        }
        for file in &entry.files {
            if file.sha256.len() != 64 || !file.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                errors.push(ValidationError::MissingSha256 {
                    repo: entry.hf_repo.clone(),
                    path: file.path.clone(),
                });
            }
            if file.size == 0 {
                errors.push(ValidationError::ZeroSize {
                    repo: entry.hf_repo.clone(),
                    path: file.path.clone(),
                });
            }
        }
        if entry.license.spdx.is_empty() {
            errors.push(ValidationError::MissingLicense {
                repo: entry.hf_repo.clone(),
            });
        }
        if !is_40_char_hex(&entry.hf_revision) {
            errors.push(ValidationError::BadRevision {
                repo: entry.hf_repo.clone(),
                revision: entry.hf_revision.clone(),
            });
        }
    }

    use std::collections::HashMap;
    let mut recommended_per_tier: HashMap<&str, usize> = HashMap::new();
    for entry in &catalog.entries {
        if !entry.tags.iter().any(|t| t == "recommended") {
            continue;
        }
        if !entry.modalities.iter().any(|m| m == "text") {
            continue;
        }
        for tag in &entry.tags {
            if let Some(tier) = tag.strip_prefix("tier:") {
                *recommended_per_tier.entry(tier).or_insert(0) += 1;
            }
        }
    }
    for (tier, count) in recommended_per_tier {
        if count != 1 {
            errors.push(ValidationError::TierRecommendedCount {
                tier: tier.to_string(),
                count,
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_has_at_least_20_entries() {
        let catalog = load();
        assert!(
            catalog.entries.len() >= 20,
            "only {} entries",
            catalog.entries.len()
        );
    }

    #[test]
    fn bundled_catalog_passes_validation() {
        let catalog = load();
        if let Err(errors) = validate(&catalog) {
            panic!("catalog.json failed validation: {errors:#?}");
        }
    }

    #[test]
    fn exactly_one_recommended_text_model_per_tier() {
        let catalog = load();
        let mut per_tier: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for entry in &catalog.entries {
            if entry.tags.iter().any(|t| t == "recommended")
                && entry.modalities.iter().any(|m| m == "text")
            {
                for tag in &entry.tags {
                    if let Some(tier) = tag.strip_prefix("tier:") {
                        *per_tier.entry(tier).or_insert(0) += 1;
                    }
                }
            }
        }
        assert_eq!(per_tier.len(), 4, "expected 4 tiers, got {per_tier:?}");
        for (tier, count) in per_tier {
            assert_eq!(count, 1, "tier {tier} has {count} recommended models");
        }
    }

    #[test]
    fn detects_a_missing_sha256() {
        let mut catalog = load();
        catalog.entries[0].files[0].sha256 = String::new();
        let errors = validate(&catalog).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ValidationError::MissingSha256 { .. })));
    }

    #[test]
    fn detects_a_short_revision() {
        let mut catalog = load();
        catalog.entries[0].hf_revision = "abc123".into();
        let errors = validate(&catalog).unwrap_err();
        assert!(errors
            .iter()
            .any(|e| matches!(e, ValidationError::BadRevision { .. })));
    }

    #[test]
    fn reasoning_defaults_false_when_absent_from_json() {
        // Pre-CORE-14 manifest JSON (e.g. rows persisted in the store) has no
        // "reasoning" key and must still parse, defaulting to false.
        let json = r#"{
            "id": "test/model@rev",
            "display_name": "Test",
            "family": "test",
            "params_b": null,
            "quant": "4bit",
            "format": "mlx",
            "modalities": ["text"],
            "context_length": 4096,
            "files": [],
            "license": {"spdx": "MIT", "commercial_ok": true, "url": ""},
            "hf_repo": "test/model",
            "hf_revision": "rev",
            "min_ram_bytes": 0,
            "est_runtime_bytes": 0,
            "tags": [],
            "curated": false
        }"#;
        let m: ModelManifest = serde_json::from_str(json).unwrap();
        assert!(!m.reasoning);
    }

    #[test]
    fn curated_entries_carry_reasoning_flag() {
        let catalog = load();
        let flag = |prefix: &str| -> bool {
            catalog
                .entries
                .iter()
                .find(|e| e.id.starts_with(prefix))
                .unwrap_or_else(|| panic!("no entry with id prefix {prefix}"))
                .reasoning
        };
        // Thinking-capable families.
        assert!(flag("mlx-community/Qwen3-0.6B-4bit"));
        assert!(flag("mlx-community/Qwen3-8B-4bit"));
        assert!(flag("mlx-community/Qwen3-14B-4bit"));
        assert!(flag("mlx-community/SmolLM3-3B-4bit"));
        assert!(flag("mlx-community/gpt-oss-20b-MXFP4-Q8"));
        // Plain instruct/chat and non-text models.
        assert!(!flag("mlx-community/Qwen3-4B-Instruct-2507-4bit"));
        assert!(!flag("mlx-community/Qwen3-30B-A3B-Instruct-2507-4bit"));
        assert!(!flag("mlx-community/Qwen3-VL-4B-Instruct-4bit"));
        assert!(!flag("mlx-community/Llama-3.2-3B-Instruct-4bit"));
        assert!(!flag("mlx-community/gemma-3-1b-it-qat-4bit"));
        assert!(!flag("argmaxinc/whisperkit-coreml"));
    }

    #[test]
    fn every_curated_entry_states_reasoning_explicitly() {
        // serde(default) would silently turn a forgotten key into false.
        let raw: serde_json::Value =
            serde_json::from_str(CATALOG_JSON).unwrap();
        for e in raw["entries"].as_array().unwrap() {
            assert!(e["reasoning"].is_boolean(), "{} lacks reasoning", e["id"]);
        }
    }

    #[test]
    fn reasoning_heuristic_matches_known_families() {
        // true: thinking families
        assert!(supports_reasoning("mlx-community/Qwen3-8B-4bit@abc"));
        assert!(supports_reasoning("mlx-community/Qwen3-0.6B-4bit"));
        assert!(supports_reasoning("Qwen/QwQ-32B"));
        assert!(supports_reasoning(
            "mlx-community/DeepSeek-R1-Distill-Qwen-7B-4bit"
        ));
        assert!(supports_reasoning("mlx-community/gpt-oss-20b-MXFP4-Q8"));
        assert!(supports_reasoning("HuggingFaceTB/SmolLM3-3B"));
        // false: Qwen3 non-thinking 2507 instruct split
        assert!(!supports_reasoning(
            "mlx-community/Qwen3-4B-Instruct-2507-4bit"
        ));
        assert!(!supports_reasoning(
            "unsloth/Qwen3-4B-Instruct-2507-GGUF@a06e"
        ));
        // false: plain instruct/chat models
        assert!(!supports_reasoning("mlx-community/Llama-3.2-3B-Instruct-4bit"));
        assert!(!supports_reasoning("mlx-community/gemma-3-1b-it-qat-4bit"));
        assert!(!supports_reasoning("mlx-community/Phi-4-mini-instruct-4bit"));
        assert!(!supports_reasoning("mlx-community/Qwen2.5-VL-7B-Instruct-4bit"));
        // false: SmolVLM2 must not match smollm3
        assert!(!supports_reasoning(
            "mlx-community/SmolVLM2-500M-Video-Instruct-mlx"
        ));
        // false: non-text
        assert!(!supports_reasoning("argmaxinc/whisperkit-coreml@0f63"));
    }

    #[test]
    fn catalog_has_valid_review_friendly_entry() {
        let catalog = load();
        let rf_entry = catalog
            .entries
            .iter()
            .find(|e| e.tags.iter().any(|t| t == "review_friendly"))
            .expect("catalog must contain a review_friendly entry");
        assert_eq!(rf_entry.hf_repo, "mlx-community/Qwen3-0.6B-4bit");
        assert_eq!(
            rf_entry.hf_revision,
            "73e3e38d981303bc594367cd910ea6eb48349da8"
        );
        assert!(!rf_entry.files.is_empty());
        let total_size: u64 = rf_entry.files.iter().map(|f| f.size).sum();
        assert!(
            total_size < 500 * 1024 * 1024,
            "review-friendly model should be compact (<500MB), got {total_size} bytes"
        );
    }
}
