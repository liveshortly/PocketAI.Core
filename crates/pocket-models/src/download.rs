use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::catalog::ModelManifest;

#[derive(Debug, Clone)]
pub struct PlannedFile {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub dest_part: PathBuf,
    pub dest_final: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DownloadPlan {
    pub files: Vec<PlannedFile>,
    pub total_bytes: u64,
    pub dest_dir: PathBuf,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DownloadError {
    #[error("needs {needed_bytes} bytes (with 10% headroom), only {available_bytes} free")]
    DiskFull {
        needed_bytes: u64,
        available_bytes: u64,
    },
    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("io error: {0}")]
    Io(String),
}

/// CORE-09: builds the file-by-file download plan for a model, refusing if
/// disk space (with 10% headroom) is insufficient. `models_dir` is the
/// `Application Support/PocketAI/Models/` root; the model lands in
/// `<models_dir>/<repo>/<rev>/`.
pub fn plan_download(
    manifest: &ModelManifest,
    models_dir: &Path,
    free_disk_bytes: u64,
) -> Result<DownloadPlan, DownloadError> {
    let total_bytes: u64 = manifest.files.iter().map(|f| f.size).sum();
    let needed = (total_bytes as f64 * 1.1) as u64;
    if free_disk_bytes < needed {
        return Err(DownloadError::DiskFull {
            needed_bytes: needed,
            available_bytes: free_disk_bytes,
        });
    }

    let dest_dir = models_dir
        .join(&manifest.hf_repo)
        .join(&manifest.hf_revision);
    let files = manifest
        .files
        .iter()
        .map(|f| PlannedFile {
            url: f.url.clone(),
            size: f.size,
            sha256: f.sha256.clone(),
            dest_part: dest_dir.join(format!("{}.part", f.path)),
            dest_final: dest_dir.join(&f.path),
        })
        .collect();

    Ok(DownloadPlan {
        files,
        total_bytes,
        dest_dir,
    })
}

/// Throttles `model.download.progress` writes to at most 2/s per model, per
/// CORE-09 ("throttled DB writes").
#[derive(Default)]
pub struct ProgressThrottle {
    last_emit: Mutex<HashMap<String, Instant>>,
}

const MIN_INTERVAL: Duration = Duration::from_millis(500);

impl ProgressThrottle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if this call should actually write/emit (caller then
    /// does the DB write + `model.download.progress` event), `false` if it's
    /// within the throttle window and should be dropped.
    pub fn should_emit(&self, model_id: &str, now: Instant) -> bool {
        let mut last = self.last_emit.lock().unwrap();
        match last.get(model_id) {
            Some(t) if now.duration_since(*t) < MIN_INTERVAL => false,
            _ => {
                last.insert(model_id.to_string(), now);
                true
            }
        }
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Verifies every `.part` file's SHA-256 against the plan, then atomically
/// renames each into place. On any mismatch, deletes that file's `.part` and
/// returns `ChecksumMismatch` without renaming the others that already
/// matched (caller marks the whole model `failed{checksum}`).
pub fn verify_and_commit(plan: &DownloadPlan) -> Result<(), DownloadError> {
    // A file with no .part but a present final was committed by an earlier,
    // interrupted pass (or a pre-relaunch run); skipping it makes this
    // idempotent so relaunch recovery can re-verify a partially-committed model.
    let already_committed =
        |f: &PlannedFile| !f.dest_part.exists() && f.dest_final.exists();
    for file in &plan.files {
        if already_committed(file) {
            continue;
        }
        let actual = sha256_file(&file.dest_part).map_err(|e| DownloadError::Io(e.to_string()))?;
        if actual != file.sha256 {
            let _ = std::fs::remove_file(&file.dest_part);
            return Err(DownloadError::ChecksumMismatch {
                path: file.dest_part.display().to_string(),
                expected: file.sha256.clone(),
                actual,
            });
        }
    }
    for file in &plan.files {
        if already_committed(file) {
            continue;
        }
        if let Some(parent) = file.dest_final.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DownloadError::Io(e.to_string()))?;
        }
        std::fs::rename(&file.dest_part, &file.dest_final)
            .map_err(|e| DownloadError::Io(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{LicenseInfo, ModelFile};

    fn manifest(files: Vec<ModelFile>) -> ModelManifest {
        ModelManifest {
            id: "mlx-community/x@rev".into(),
            display_name: "x".into(),
            family: "x".into(),
            params_b: None,
            quant: "4bit".into(),
            format: "mlx".into(),
            modalities: vec!["text".into()],
            context_length: 4096,
            files,
            license: LicenseInfo {
                spdx: "MIT".into(),
                commercial_ok: true,
                url: String::new(),
            },
            hf_repo: "mlx-community/x".into(),
            hf_revision: "rev".into(),
            min_ram_bytes: 0,
            est_runtime_bytes: 0,
            tags: vec![],
            curated: true,
        }
    }

    fn file(path: &str, size: u64, sha256: &str) -> ModelFile {
        ModelFile {
            path: path.into(),
            size,
            sha256: sha256.into(),
            url: format!("https://huggingface.co/mlx-community/x/resolve/rev/{path}"),
        }
    }

    #[test]
    fn plans_download_with_correct_dest_paths_and_total() {
        let m = manifest(vec![file(
            "model.safetensors",
            1000,
            "a".repeat(64).as_str(),
        )]);
        let dir = tempfile::tempdir().unwrap();
        let plan = plan_download(&m, dir.path(), 10_000).unwrap();
        assert_eq!(plan.total_bytes, 1000);
        assert_eq!(
            plan.dest_dir,
            dir.path().join("mlx-community/x").join("rev")
        );
        assert!(plan.files[0]
            .dest_part
            .to_string_lossy()
            .ends_with("model.safetensors.part"));
        assert!(plan.files[0]
            .dest_final
            .to_string_lossy()
            .ends_with("model.safetensors"));
    }

    #[test]
    fn refuses_when_disk_too_full() {
        let m = manifest(vec![file(
            "model.safetensors",
            1_000_000,
            "a".repeat(64).as_str(),
        )]);
        let dir = tempfile::tempdir().unwrap();
        let err = plan_download(&m, dir.path(), 500_000).unwrap_err();
        assert!(matches!(err, DownloadError::DiskFull { .. }));
    }

    #[test]
    fn verify_and_commit_succeeds_on_matching_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"hello world";
        let expected_sha = sha256_hex(content);
        let m = manifest(vec![file("w.bin", content.len() as u64, &expected_sha)]);
        let plan = plan_download(&m, dir.path(), u64::MAX).unwrap();

        std::fs::create_dir_all(&plan.dest_dir).unwrap();
        std::fs::write(&plan.files[0].dest_part, content).unwrap();

        verify_and_commit(&plan).unwrap();
        assert!(plan.files[0].dest_final.exists());
        assert!(!plan.files[0].dest_part.exists());
    }

    #[test]
    fn verify_and_commit_skips_already_committed_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = b"first file";
        let b = b"second file";
        let m = manifest(vec![
            file("a.bin", a.len() as u64, &sha256_hex(a)),
            file("b.bin", b.len() as u64, &sha256_hex(b)),
        ]);
        let plan = plan_download(&m, dir.path(), u64::MAX).unwrap();

        std::fs::create_dir_all(&plan.dest_dir).unwrap();
        // a.bin was committed by an earlier interrupted pass: final exists, no part.
        std::fs::write(&plan.files[0].dest_final, a).unwrap();
        // b.bin still awaits verification.
        std::fs::write(&plan.files[1].dest_part, b).unwrap();

        verify_and_commit(&plan).unwrap();
        assert!(plan.files[0].dest_final.exists());
        assert!(plan.files[1].dest_final.exists());
        assert!(!plan.files[1].dest_part.exists());
    }

    #[test]
    fn verify_and_commit_rejects_checksum_mismatch_and_removes_part() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"hello world";
        let wrong_sha = "0".repeat(64);
        let m = manifest(vec![file("w.bin", content.len() as u64, &wrong_sha)]);
        let plan = plan_download(&m, dir.path(), u64::MAX).unwrap();

        std::fs::create_dir_all(&plan.dest_dir).unwrap();
        std::fs::write(&plan.files[0].dest_part, content).unwrap();

        let err = verify_and_commit(&plan).unwrap_err();
        assert!(matches!(err, DownloadError::ChecksumMismatch { .. }));
        assert!(!plan.files[0].dest_part.exists());
        assert!(!plan.files[0].dest_final.exists());
    }

    #[test]
    fn resume_after_truncation_reports_bytes_done_via_part_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let full_content = b"0123456789";
        let m = manifest(vec![file(
            "w.bin",
            full_content.len() as u64,
            &sha256_hex(full_content),
        )]);
        let plan = plan_download(&m, dir.path(), u64::MAX).unwrap();
        std::fs::create_dir_all(&plan.dest_dir).unwrap();

        // Simulate a truncated prior download: only the first half landed.
        std::fs::write(&plan.files[0].dest_part, &full_content[..5]).unwrap();
        let bytes_done = std::fs::metadata(&plan.files[0].dest_part).unwrap().len();
        assert_eq!(bytes_done, 5);

        // Resume appends the rest; verify_and_commit only runs once complete.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&plan.files[0].dest_part)
            .unwrap();
        f.write_all(&full_content[5..]).unwrap();
        drop(f);

        verify_and_commit(&plan).unwrap();
        assert!(plan.files[0].dest_final.exists());
    }

    #[test]
    fn progress_throttle_allows_at_most_two_per_second() {
        let throttle = ProgressThrottle::new();
        let t0 = Instant::now();
        assert!(throttle.should_emit("m1", t0));
        assert!(!throttle.should_emit("m1", t0 + Duration::from_millis(100)));
        assert!(throttle.should_emit("m1", t0 + Duration::from_millis(600)));
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
