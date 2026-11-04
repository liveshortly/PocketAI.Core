use crate::repos::settings::SettingsRepo;
use crate::Store;

/// Typed getters/setters over `SettingsRepo` (CORE-12). Every key in
/// `01-architecture.md`'s settings list, with a sensible default when unset.
pub struct TypedSettings<'a> {
    repo: SettingsRepo<'a>,
}

fn parse_bool(s: &str) -> bool {
    s == "true"
}

impl<'a> TypedSettings<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self {
            repo: SettingsRepo::new(store),
        }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.repo.get(key).ok().flatten()
    }

    fn set(&self, key: &str, value: String) {
        let _ = self.repo.set(key, &value);
    }

    pub fn offline_mode(&self) -> bool {
        self.get("offline_mode")
            .map(|v| parse_bool(&v))
            .unwrap_or(false)
    }
    pub fn set_offline_mode(&self, on: bool) {
        self.set("offline_mode", on.to_string());
    }

    /// Fraction of total unified memory a Mac may use for models (0.50-0.85).
    pub fn memory_policy_fraction(&self) -> f32 {
        self.get("memory_policy_fraction")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.70)
    }
    pub fn set_memory_policy_fraction(&self, fraction: f32) {
        self.set("memory_policy_fraction", fraction.to_string());
    }

    pub fn allow_unfit_models(&self) -> bool {
        self.get("allow_unfit_models")
            .map(|v| parse_bool(&v))
            .unwrap_or(false)
    }
    pub fn set_allow_unfit_models(&self, allow: bool) {
        self.set("allow_unfit_models", allow.to_string());
    }

    pub fn default_model(&self, modality: &str) -> Option<String> {
        self.get(&format!("default_model.{modality}"))
    }
    pub fn set_default_model(&self, modality: &str, model_id: &str) {
        self.set(&format!("default_model.{modality}"), model_id.to_string());
    }

    pub fn server_enabled(&self) -> bool {
        self.get("server.enabled")
            .map(|v| parse_bool(&v))
            .unwrap_or(false)
    }
    pub fn set_server_enabled(&self, on: bool) {
        self.set("server.enabled", on.to_string());
    }

    pub fn server_lan(&self) -> bool {
        self.get("server.lan")
            .map(|v| parse_bool(&v))
            .unwrap_or(false)
    }
    pub fn set_server_lan(&self, on: bool) {
        self.set("server.lan", on.to_string());
    }

    /// AGT-13 stage 2: "Let my paired iPhone run code on this Mac". Off until
    /// the user turns it on; the run endpoint refuses everything while off.
    pub fn run_code_enabled(&self) -> bool {
        self.get("server.run_code_enabled")
            .map(|v| parse_bool(&v))
            .unwrap_or(false)
    }
    pub fn set_run_code_enabled(&self, on: bool) {
        self.set("server.run_code_enabled", on.to_string());
    }

    pub fn server_port(&self) -> u16 {
        self.get("server.port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(7480)
    }
    pub fn set_server_port(&self, port: u16) {
        self.set("server.port", port.to_string());
    }

    pub fn server_local_api_key_hash(&self) -> Option<String> {
        self.get("server.local_api_key_hash")
    }
    pub fn set_server_local_api_key_hash(&self, hash: &str) {
        self.set("server.local_api_key_hash", hash.to_string());
    }

    /// MAC-21: this host's stable pairing id (created once, never a path).
    pub fn host_id(&self) -> Option<String> {
        self.get("host.id")
    }
    pub fn set_host_id(&self, id: &str) {
        self.set("host.id", id.to_string());
    }

    pub fn crash_reports_opt_in(&self) -> bool {
        self.get("crash_reports_opt_in")
            .map(|v| parse_bool(&v))
            .unwrap_or(false)
    }
    pub fn set_crash_reports_opt_in(&self, on: bool) {
        self.set("crash_reports_opt_in", on.to_string());
    }

    pub fn catalog_last_refresh(&self) -> Option<i64> {
        self.get("catalog_last_refresh")
            .and_then(|v| v.parse().ok())
    }
    pub fn set_catalog_last_refresh(&self, ts_ms: i64) {
        self.set("catalog_last_refresh", ts_ms.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_unset() {
        let store = Store::open_in_memory().unwrap();
        let settings = TypedSettings::new(&store);
        assert!(!settings.offline_mode());
        assert_eq!(settings.memory_policy_fraction(), 0.70);
        assert!(!settings.allow_unfit_models());
        assert_eq!(settings.default_model("text"), None);
        assert!(!settings.server_enabled());
        assert!(!settings.server_lan());
        assert!(!settings.run_code_enabled());
        assert_eq!(settings.server_port(), 7480);
        assert_eq!(settings.server_local_api_key_hash(), None);
        assert!(!settings.crash_reports_opt_in());
        assert_eq!(settings.catalog_last_refresh(), None);
    }

    #[test]
    fn set_then_get_round_trips_every_key() {
        let store = Store::open_in_memory().unwrap();
        let settings = TypedSettings::new(&store);

        settings.set_offline_mode(true);
        assert!(settings.offline_mode());

        settings.set_memory_policy_fraction(0.85);
        assert_eq!(settings.memory_policy_fraction(), 0.85);

        settings.set_allow_unfit_models(true);
        assert!(settings.allow_unfit_models());

        settings.set_default_model("text", "mlx-community/Qwen3-4B-Instruct-2507-4bit@rev");
        assert_eq!(
            settings.default_model("text"),
            Some("mlx-community/Qwen3-4B-Instruct-2507-4bit@rev".to_string())
        );
        assert_eq!(settings.default_model("vision"), None);

        settings.set_server_enabled(true);
        assert!(settings.server_enabled());
        settings.set_server_lan(true);
        assert!(settings.server_lan());
        settings.set_server_port(7481);
        assert_eq!(settings.server_port(), 7481);
        settings.set_server_local_api_key_hash("abc123");
        assert_eq!(
            settings.server_local_api_key_hash(),
            Some("abc123".to_string())
        );

        settings.set_crash_reports_opt_in(true);
        assert!(settings.crash_reports_opt_in());

        settings.set_catalog_last_refresh(1760400000000);
        assert_eq!(settings.catalog_last_refresh(), Some(1760400000000));
    }
}
