use serde::Deserialize;

const CHIPS_JSON: &str = include_str!("../data/chips.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ChipSpec {
    pub chip: String,
    pub bandwidth_gbps: f32,
    pub gpu_cores: u32,
    pub confidence: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChipEntry {
    #[serde(default)]
    identifiers: Vec<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(flatten)]
    spec: ChipSpec,
}

#[derive(Debug, Clone, Deserialize)]
struct ChipTable {
    chips: Vec<ChipEntry>,
    fallback: ChipSpec,
}

fn table() -> ChipTable {
    serde_json::from_str(CHIPS_JSON).expect("bundled chips.json must parse")
}

/// Resolves a `hw.model`/`machine` sysctl identifier (e.g. "iPhone17,1",
/// "Mac16,10") to its chip's memory bandwidth table entry. Unknown
/// identifiers get the conservative fallback bucket (CORE-07/08: "unknown
/// chips -> Estimated, conservative bucket").
pub fn resolve(model_identifier: &str) -> ChipSpec {
    let t = table();
    for entry in &t.chips {
        if entry.identifiers.iter().any(|id| id == model_identifier) {
            return entry.spec.clone();
        }
        if let Some(prefix) = &entry.prefix {
            if model_identifier.starts_with(prefix.as_str()) {
                return entry.spec.clone();
            }
        }
    }
    t.fallback.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_json_parses() {
        let t = table();
        assert!(!t.chips.is_empty());
    }

    #[test]
    fn resolves_iphone17_1_to_a18_pro() {
        let spec = resolve("iPhone17,1");
        assert_eq!(spec.chip, "A18 Pro");
        assert!(spec.bandwidth_gbps > 0.0);
    }

    #[test]
    fn resolves_mac16_10_to_m4() {
        let spec = resolve("Mac16,10");
        assert_eq!(spec.chip, "M4");
    }

    #[test]
    fn resolves_m5_generation_by_prefix() {
        let spec = resolve("Mac17,1");
        assert_eq!(spec.chip, "M5");
    }

    #[test]
    fn unknown_identifier_falls_back_to_conservative_default() {
        let spec = resolve("SomeFutureDevice99,1");
        assert_eq!(spec.chip, "Unknown");
        assert_eq!(spec.confidence, "conservative_default");
    }

    #[test]
    fn every_entry_has_a_source() {
        let t = table();
        for entry in &t.chips {
            assert!(!entry.spec.source.is_empty(), "{} has no source", entry.spec.chip);
            assert!(entry.spec.bandwidth_gbps > 0.0, "{} has zero bandwidth", entry.spec.chip);
        }
    }
}
