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
}
