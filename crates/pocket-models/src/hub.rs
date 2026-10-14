use serde::Deserialize;

use crate::catalog::{LicenseInfo, ModelManifest};
use pocket_trust::PocketHttp;

#[derive(Debug, Deserialize)]
struct HubModel {
    id: String,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

/// CORE-10: user-initiated Hugging Face search, via `PocketHttp` so it's
/// egress-recorded and refused in offline mode. Results are uncurated
/// (`curated=false`); fit isn't computed here - that needs `config.json`,
/// fetched lazily when the user opens a result's detail (the FFI layer
/// wraps these manifests into a `CatalogEntry` with a placeholder
/// "open to check fit" report).
///
/// `base_url` defaults to `"https://huggingface.co"` in production; tests
/// point it at a local mock server so there's no live network dependency.
pub async fn search_hub(
    http: &PocketHttp,
    base_url: &str,
    query: &str,
    format: &str,
) -> Result<Vec<ModelManifest>, pocket_types::PocketError> {
    let url = format!(
        "{base_url}/api/models?search={}&filter={format}&sort=downloads&limit=30",
        urlencode(query)
    );
    let bytes = http.get(&url, "catalog_search", "app").await?;
    let models: Vec<HubModel> =
        serde_json::from_slice(&bytes).map_err(|e| pocket_types::PocketError::Network {
            message: format!("hub search response did not parse: {e}"),
        })?;

    Ok(models
        .into_iter()
        .map(|m| {
            let rev = m.sha.unwrap_or_else(|| "main".to_string());
            let display_name = m.id.split('/').next_back().unwrap_or(&m.id).to_string();
            ModelManifest {
                id: format!("{}@{rev}", m.id),
                display_name,
                family: m.pipeline_tag.unwrap_or_default(),
                params_b: None,
                quant: "unknown".into(),
                format: format.into(),
                modalities: vec!["text".into()],
                context_length: 4096,
                files: vec![],
                license: LicenseInfo {
                    spdx: "unknown".into(),
                    commercial_ok: false,
                    url: format!("https://huggingface.co/{}", m.id),
                },
                hf_repo: m.id,
                hf_revision: rev,
                min_ram_bytes: 0,
                est_runtime_bytes: 0,
                tags: vec![],
                curated: false,
            }
        })
        .collect())
}

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Minimal local HTTP/1.1 server returning a fixed, recorded-looking HF
    /// search response body - no live network touched.
    fn spawn_mock_hub(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    const RECORDED_RESPONSE: &str = r#"[
        {"id": "mlx-community/Qwen3-0.6B-4bit", "sha": "abc123def456", "pipeline_tag": "text-generation"},
        {"id": "mlx-community/SomeOtherModel", "pipeline_tag": "text-generation"}
    ]"#;

    #[tokio::test]
    async fn parses_a_recorded_hub_response_into_uncurated_manifests() {
        let base_url = spawn_mock_hub(RECORDED_RESPONSE);
        let http = PocketHttp::new();

        let entries = search_hub(&http, &base_url, "qwen3", "mlx").await.unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "mlx-community/Qwen3-0.6B-4bit@abc123def456");
        assert!(!entries[0].curated);
        // No sha returned -> falls back to "main".
        assert_eq!(entries[1].hf_revision, "main");
    }

    #[tokio::test]
    async fn refuses_when_offline() {
        let http = PocketHttp::new();
        http.set_offline_mode(true);
        let err = search_hub(&http, "http://example.invalid", "qwen3", "mlx")
            .await
            .unwrap_err();
        assert!(matches!(err, pocket_types::PocketError::Network { .. }));
    }
}
