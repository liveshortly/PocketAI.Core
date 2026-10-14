mod catalog;
mod chips;
mod download;
mod fit;

pub use catalog::{load, validate, Catalog, LicenseInfo, ModelFile, ModelManifest, ValidationError};
pub use chips::{resolve as resolve_chip, ChipSpec};
pub use download::{plan_download, verify_and_commit, DownloadError, DownloadPlan, PlannedFile, ProgressThrottle};
pub use fit::{fit, Benchmark, Confidence, FitReport, FitVerdict, KvShape, Platform};
