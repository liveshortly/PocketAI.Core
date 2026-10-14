#[cfg(test)]
mod tests {
    use crate::core::PocketCore;
    use crate::traits::PlatformHooks;
    use crate::types::*;
    use std::sync::Arc;

    struct FakeHooks {
        dir: String,
    }

    impl PlatformHooks for FakeHooks {
        fn keychain_get(&self, _key: String) -> Option<Vec<u8>> {
            None
        }
        fn keychain_set(&self, _key: String, _value: Vec<u8>) -> bool {
            true
        }
        fn device_profile(&self) -> DeviceProfile {
            DeviceProfile {
                platform: Platform::Ios,
                model_identifier: "iPhone17,1".into(),
                chip: "A18 Pro".into(),
                total_ram_bytes: 8 * 1024 * 1024 * 1024,
                app_memory_limit_bytes: 4_600_000_000,
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
            1_760_400_000_000
        }
    }

    fn open_core() -> Arc<PocketCore> {
        let dir = tempfile::tempdir().unwrap().keep();
        let hooks = FakeHooks {
            dir: dir.to_string_lossy().to_string(),
        };
        PocketCore::open(
            CoreConfig {
                app_support_dir: hooks.dir.clone(),
                platform: Platform::Ios,
            },
            Arc::new(hooks),
        )
        .unwrap()
    }

    #[test]
    fn catalog_returns_real_entries_with_fit_reports() {
        let core = open_core();
        let entries = core.catalog(CatalogFilter {
            modality: None,
            curated_only: false,
            installed_only: false,
        });
        assert!(entries.len() >= 20, "expected the real bundled catalog, got {}", entries.len());
        assert!(entries.iter().any(|e| e.manifest.tags.contains(&"recommended".to_string())));
    }

    #[test]
    fn catalog_filters_by_modality() {
        let core = open_core();
        let vision_only = core.catalog(CatalogFilter {
            modality: Some(Modality::Vision),
            curated_only: false,
            installed_only: false,
        });
        assert!(!vision_only.is_empty());
        assert!(vision_only.iter().all(|e| e.manifest.modalities.contains(&Modality::Vision)));
    }

    #[test]
    fn create_list_and_delete_conversation_round_trips() {
        let core = open_core();
        let created = core
            .create_conversation(NewConversation {
                title: Some("Test chat".into()),
                project_id: None,
                model_id: None,
            })
            .unwrap();
        assert_eq!(created.title, Some("Test chat".into()));

        let list = core.conversations(ConversationQuery {
            search: None,
            archived: None,
            limit: 50,
            offset: 0,
        });
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, created.id);

        core.delete_conversation(created.id.clone()).unwrap();
        let after = core.conversations(ConversationQuery {
            search: None,
            archived: None,
            limit: 50,
            offset: 0,
        });
        assert!(after.is_empty());
    }

    #[test]
    fn audit_log_starts_empty() {
        let core = open_core();
        let entries = core.audit_log(AuditQuery { limit: 10, since_ts: None });
        assert!(entries.is_empty());
    }

    #[test]
    fn devices_empty_then_revoke_errors_gracefully_on_missing_device() {
        let core = open_core();
        assert!(core.devices().is_empty());
        // Revoking a nonexistent device must not panic; repo update on 0 rows is Ok.
        assert!(core.revoke_device("nonexistent".into()).is_ok());
    }
}
