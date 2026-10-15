//! CI/test headless server harness for OpenAI SDK compatibility tests (QA-07).
//! Starts PocketCore with a deterministic MockEngine on loopback (127.0.0.1)
//! to avoid TLS cert setup in CI (production uses pinned certs).
use std::io::Write;
use std::sync::Arc;

use pocket_engine::MockEngine;
use pocket_ffi::{
    CoreConfig, DeviceProfile, Platform, PlatformHooks, PocketCore, PocketEngineAdapter,
    ServerConfig, ThermalState,
};

struct TestPlatformHooks {
    dir: String,
}

impl PlatformHooks for TestPlatformHooks {
    fn keychain_get(&self, _key: String) -> Option<Vec<u8>> {
        None
    }
    fn keychain_set(&self, _key: String, _value: Vec<u8>) -> bool {
        true
    }
    fn device_profile(&self) -> DeviceProfile {
        DeviceProfile {
            platform: Platform::MacOs,
            model_identifier: "Mac16,10".into(),
            chip: "M4".into(),
            total_ram_bytes: 16 * 1024 * 1024 * 1024,
            app_memory_limit_bytes: 8 * 1024 * 1024 * 1024,
            gpu_family: "metal3".into(),
            free_disk_bytes: 100 * 1024 * 1024 * 1024,
            thermal_state: ThermalState::Nominal,
            low_power_mode: false,
            has_increased_memory_entitlement: true,
        }
    }
    fn app_support_dir(&self) -> String {
        self.dir.clone()
    }
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = std::env::temp_dir().join(format!("pocket-sdk-test-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&temp_dir)?;
    let dir_str = temp_dir.to_string_lossy().to_string();

    let hooks = Arc::new(TestPlatformHooks {
        dir: dir_str.clone(),
    });
    let core = PocketCore::open(
        CoreConfig {
            app_support_dir: dir_str,
            platform: Platform::MacOs,
        },
        hooks,
    )?;

    // Register deterministic mock engine via production PocketEngineAdapter
    let mock = MockEngine::new(5000.0);
    let adapter = Arc::new(PocketEngineAdapter::new(mock));
    core.register_engine(adapter);

    // Seed installed models into database so /v1/models and chat/embeddings find them
    let db_path = temp_dir.join("pocket.sqlite");
    let store = pocket_store::Store::open(&db_path)?;
    let models_repo = pocket_store::ModelsRepo::new(&store);

    let catalog = pocket_models::load();
    for entry in catalog.entries.iter().take(3) {
        models_repo.upsert(&pocket_store::ModelRow {
            id: entry.id.clone(),
            manifest_json: serde_json::to_string(entry)?,
            state: "installed".into(),
            bytes_done: 1000,
            installed_at: Some(1760400000000),
            last_used_at: None,
            pinned_engine: None,
            benchmark_json: None,
        })?;
    }

    // Generate local API key
    let api_key = core.generate_local_api_key();

    // Start server on loopback HTTP (127.0.0.1) for tests to stay hermetic without TLS cert setup
    let status = core.server_start(ServerConfig {
        lan_enabled: false,
        port: 0,
    })?;

    println!("READY {} {}", status.port, api_key);
    std::io::stdout().flush()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        tokio::signal::ctrl_c().await.ok();
    });

    core.server_stop()?;
    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}
