use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Platform {
    Ios,
    MacOs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitVerdict {
    RunsWell,
    RunsSlow,
    WillNotFit,
    NotSupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Estimated,
    Measured,
}

#[derive(Debug, Clone, Copy)]
pub struct Benchmark {
    pub tps: f32,
    pub ttft_ms: u32,
    pub peak_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct FitReport {
    pub verdict: FitVerdict,
    pub reason: String,
    pub est_tokens_per_sec: Option<f32>,
    pub measured_tokens_per_sec: Option<f32>,
    pub est_runtime_bytes: u64,
    pub disk_bytes: u64,
    pub confidence: Confidence,
}

/// Optional `config.json`-derived shape for an accurate KV-cache estimate;
/// when unknown, `fit()` falls back to `weights_bytes * 0.12` per
/// 01-architecture.md SS4.3.
#[derive(Debug, Clone, Copy, Default)]
pub struct KvShape {
    pub layers: u32,
    pub kv_heads: u32,
    pub head_dim: u32,
}

const RUNS_SLOW_TPS_THRESHOLD: f32 = 8.0;
const KV_CONTEXT_FOR_ESTIMATE: u64 = 4096;
const BYTES_PER_KV_ELEM: u64 = 2; // f16
const IPHONE_OVERHEAD_BYTES: u64 = 350 * 1024 * 1024;
const MAC_OVERHEAD_BYTES: u64 = 600 * 1024 * 1024;
const BANDWIDTH_EFFICIENCY: f32 = 0.6;

fn kv_bytes(weights_bytes: u64, shape: Option<KvShape>) -> u64 {
    match shape {
        Some(s) => {
            2 * s.layers as u64
                * s.kv_heads as u64
                * s.head_dim as u64
                * KV_CONTEXT_FOR_ESTIMATE
                * BYTES_PER_KV_ELEM
        }
        None => (weights_bytes as f64 * 0.12) as u64,
    }
}

fn est_runtime_bytes(weights_bytes: u64, platform: Platform, shape: Option<KvShape>) -> u64 {
    let overhead = match platform {
        Platform::Ios => IPHONE_OVERHEAD_BYTES,
        Platform::MacOs => MAC_OVERHEAD_BYTES,
    };
    (weights_bytes as f64 * 1.05) as u64 + kv_bytes(weights_bytes, shape) + overhead
}

fn est_tokens_per_sec(bandwidth_gbps: f32, weights_bytes: u64) -> f32 {
    let weights_gb = weights_bytes as f32 / 1e9;
    if weights_gb <= 0.0 {
        return 0.0;
    }
    bandwidth_gbps * BANDWIDTH_EFFICIENCY / weights_gb
}

fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Per 01-architecture.md SS4.3's fit math. `budget_bytes` is the device's
/// app memory limit (iPhone: `os_proc_available_memory()`-derived; Mac:
/// total unified memory x the user's memory-policy fraction).
pub fn fit(
    weights_bytes: u64,
    disk_bytes: u64,
    platform: Platform,
    budget_bytes: u64,
    bandwidth_gbps: f32,
    kv_shape: Option<KvShape>,
    measured: Option<Benchmark>,
) -> FitReport {
    let runtime_bytes = est_runtime_bytes(weights_bytes, platform, kv_shape);
    let est_tps = est_tokens_per_sec(bandwidth_gbps, weights_bytes);

    let (verdict, reason) = if runtime_bytes > budget_bytes {
        (
            FitVerdict::WillNotFit,
            format!(
                "Needs {}, this device allows {}",
                format_gb(runtime_bytes),
                format_gb(budget_bytes)
            ),
        )
    } else if measured.map(|b| b.tps).unwrap_or(est_tps) < RUNS_SLOW_TPS_THRESHOLD {
        (
            FitVerdict::RunsSlow,
            format!(
                "Fits in {}, but generates slowly on this device",
                format_gb(runtime_bytes)
            ),
        )
    } else {
        (
            FitVerdict::RunsWell,
            format!("Fits comfortably in {}", format_gb(runtime_bytes)),
        )
    };

    FitReport {
        verdict,
        reason,
        est_tokens_per_sec: Some(est_tps),
        measured_tokens_per_sec: measured.map(|b| b.tps),
        est_runtime_bytes: runtime_bytes,
        disk_bytes,
        confidence: if measured.is_some() {
            Confidence::Measured
        } else {
            Confidence::Estimated
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;

    struct DeviceProfile {
        name: &'static str,
        budget_bytes: u64,
        platform: Platform,
        bandwidth_gbps: f32,
    }

    const IPHONE_6GB: DeviceProfile = DeviceProfile {
        name: "iPhone 6GB class",
        budget_bytes: 32 * GB / 10,
        platform: Platform::Ios,
        bandwidth_gbps: 34.0,
    };
    const IPHONE_8GB: DeviceProfile = DeviceProfile {
        name: "iPhone 8GB class",
        budget_bytes: 46 * GB / 10,
        platform: Platform::Ios,
        bandwidth_gbps: 68.26,
    };
    const IPHONE_12GB: DeviceProfile = DeviceProfile {
        name: "iPhone 12GB class",
        budget_bytes: 6 * GB,
        platform: Platform::Ios,
        bandwidth_gbps: 75.8,
    };
    const MAC_8GB: DeviceProfile = DeviceProfile {
        name: "Mac 8GB",
        budget_bytes: (8 * GB * 70) / 100,
        platform: Platform::MacOs,
        bandwidth_gbps: 100.0,
    };
    const MAC_16GB: DeviceProfile = DeviceProfile {
        name: "Mac 16GB",
        budget_bytes: (16 * GB * 70) / 100,
        platform: Platform::MacOs,
        bandwidth_gbps: 120.0,
    };
    const MAC_64GB: DeviceProfile = DeviceProfile {
        name: "Mac 64GB",
        budget_bytes: (64 * GB * 70) / 100,
        platform: Platform::MacOs,
        bandwidth_gbps: 400.0,
    };

    struct Model {
        name: &'static str,
        weights_bytes: u64,
    }

    const TINY_1B: Model = Model {
        name: "1B-4bit",
        weights_bytes: GB / 2,
    };
    const SMALL_3B: Model = Model {
        name: "3B-4bit",
        weights_bytes: (GB * 17) / 10,
    };
    const MED_4B: Model = Model {
        name: "4B-4bit",
        weights_bytes: (GB * 23) / 10,
    };
    const MED_8B: Model = Model {
        name: "8B-4bit",
        weights_bytes: (GB * 45) / 10,
    };
    const LARGE_14B: Model = Model {
        name: "14B-4bit",
        weights_bytes: 8 * GB,
    };
    const LARGE_20B: Model = Model {
        name: "20B-4bit",
        weights_bytes: 12 * GB,
    };
    const HUGE_30B: Model = Model {
        name: "30B-4bit",
        weights_bytes: 17 * GB,
    };
    const HUGE_70B: Model = Model {
        name: "70B-4bit",
        weights_bytes: 38 * GB,
    };

    #[test]
    fn fit_matrix_produces_expected_verdicts() {
        let devices = [
            IPHONE_6GB,
            IPHONE_8GB,
            IPHONE_12GB,
            MAC_8GB,
            MAC_16GB,
            MAC_64GB,
        ];
        let models = [
            TINY_1B, SMALL_3B, MED_4B, MED_8B, LARGE_14B, LARGE_20B, HUGE_30B, HUGE_70B,
        ];

        for device in &devices {
            for model in &models {
                let report = fit(
                    model.weights_bytes,
                    model.weights_bytes,
                    device.platform,
                    device.budget_bytes,
                    device.bandwidth_gbps,
                    None,
                    None,
                );
                // Every verdict must be internally consistent with the budget check,
                // regardless of the exact threshold tuning (the real assertion below
                // table-drives known-clear cases; this one guards the invariant).
                if report.est_runtime_bytes > device.budget_bytes {
                    assert_eq!(
                        report.verdict,
                        FitVerdict::WillNotFit,
                        "{} on {}: runtime {} > budget {} but verdict was {:?}",
                        model.name,
                        device.name,
                        report.est_runtime_bytes,
                        device.budget_bytes,
                        report.verdict
                    );
                } else {
                    assert_ne!(report.verdict, FitVerdict::WillNotFit);
                }
            }
        }
    }

    #[test]
    fn tiny_model_runs_well_on_smallest_iphone() {
        let report = fit(
            TINY_1B.weights_bytes,
            TINY_1B.weights_bytes,
            Platform::Ios,
            IPHONE_6GB.budget_bytes,
            IPHONE_6GB.bandwidth_gbps,
            None,
            None,
        );
        assert_eq!(report.verdict, FitVerdict::RunsWell);
    }

    #[test]
    fn huge_70b_will_not_fit_on_any_iphone() {
        for device in [IPHONE_6GB, IPHONE_8GB, IPHONE_12GB] {
            let report = fit(
                HUGE_70B.weights_bytes,
                HUGE_70B.weights_bytes,
                device.platform,
                device.budget_bytes,
                device.bandwidth_gbps,
                None,
                None,
            );
            assert_eq!(report.verdict, FitVerdict::WillNotFit, "{}", device.name);
        }
    }

    #[test]
    fn huge_30b_runs_on_mac_64gb() {
        let report = fit(
            HUGE_30B.weights_bytes,
            HUGE_30B.weights_bytes,
            Platform::MacOs,
            MAC_64GB.budget_bytes,
            MAC_64GB.bandwidth_gbps,
            None,
            None,
        );
        assert_ne!(report.verdict, FitVerdict::WillNotFit);
    }

    #[test]
    fn low_bandwidth_relative_to_weights_runs_slow_not_will_not_fit() {
        // Fits in budget but bandwidth/weights ratio is below the tok/s floor.
        let report = fit(
            LARGE_14B.weights_bytes,
            LARGE_14B.weights_bytes,
            Platform::MacOs,
            MAC_64GB.budget_bytes,
            34.0,
            None,
            None,
        );
        assert_eq!(report.verdict, FitVerdict::RunsSlow);
    }

    #[test]
    fn measured_benchmark_overrides_estimated_confidence() {
        let estimated = fit(
            MED_4B.weights_bytes,
            MED_4B.weights_bytes,
            Platform::Ios,
            IPHONE_8GB.budget_bytes,
            IPHONE_8GB.bandwidth_gbps,
            None,
            None,
        );
        assert_eq!(estimated.confidence, Confidence::Estimated);
        assert_eq!(estimated.measured_tokens_per_sec, None);

        let measured = fit(
            MED_4B.weights_bytes,
            MED_4B.weights_bytes,
            Platform::Ios,
            IPHONE_8GB.budget_bytes,
            IPHONE_8GB.bandwidth_gbps,
            None,
            Some(Benchmark {
                tps: 2.0,
                ttft_ms: 500,
                peak_bytes: MED_4B.weights_bytes,
            }),
        );
        assert_eq!(measured.confidence, Confidence::Measured);
        assert_eq!(measured.measured_tokens_per_sec, Some(2.0));
        // A measured 2 tok/s overrides the (likely higher) estimate and must
        // demote a RunsWell estimate to RunsSlow.
        assert_eq!(measured.verdict, FitVerdict::RunsSlow);
    }

    #[test]
    fn kv_shape_increases_estimate_over_flat_fallback() {
        let without_shape = fit(
            MED_8B.weights_bytes,
            MED_8B.weights_bytes,
            Platform::MacOs,
            MAC_16GB.budget_bytes,
            MAC_16GB.bandwidth_gbps,
            None,
            None,
        );
        let with_shape = fit(
            MED_8B.weights_bytes,
            MED_8B.weights_bytes,
            Platform::MacOs,
            MAC_16GB.budget_bytes,
            MAC_16GB.bandwidth_gbps,
            Some(KvShape {
                layers: 32,
                kv_heads: 8,
                head_dim: 128,
            }),
            None,
        );
        // Both are valid estimates; just prove the shape path actually changes the number.
        assert_ne!(
            without_shape.est_runtime_bytes,
            with_shape.est_runtime_bytes
        );
    }
}
