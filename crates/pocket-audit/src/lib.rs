use std::io::Write;

use pocket_store::{AuditRepo, Store};
use pocket_types::PocketError;

/// Domain service over `pocket_store::AuditRepo` (NET-14). The repo already
/// implements the hash chain (CORE-03); this crate is the audit-log-specific
/// API consumers reach for: logging an action, verifying the chain, and
/// exporting JSONL.
pub struct AuditLog<'a> {
    repo: AuditRepo<'a>,
}

impl<'a> AuditLog<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self {
            repo: AuditRepo::new(store),
        }
    }

    /// Logs one of the actions NET-14 enumerates: model load/unload/delete,
    /// server on/off, LAN on/off, pairing, revoke, offline mode toggle,
    /// license activation (key serial only - never the raw key), purchases
    /// (no receipts), Privacy settings changes.
    pub fn audit(
        &self,
        actor: &str,
        action: &str,
        target: &str,
        detail_json: &str,
    ) -> Result<i64, PocketError> {
        self.repo
            .append(now_ms(), actor, action, target, detail_json)
            .map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })
    }

    /// Returns the id of the first row whose hash chain breaks, or `None`.
    pub fn verify(&self) -> Result<Option<i64>, PocketError> {
        self.repo.verify_chain().map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })
    }

    pub fn export_jsonl(&self, path: &str) -> Result<(), PocketError> {
        let rows = self.repo.list(u32::MAX).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;
        let mut file = std::fs::File::create(path).map_err(|e| PocketError::Storage {
            message: e.to_string(),
        })?;
        for row in rows {
            let line = serde_json::json!({
                "id": row.id,
                "ts": row.ts,
                "actor": row.actor,
                "action": row.action,
                "target": row.target,
                "detail": serde_json::from_str::<serde_json::Value>(&row.detail_json).unwrap_or(serde_json::Value::Null),
                "hash": hex::encode(&row.hash),
            });
            writeln!(file, "{line}").map_err(|e| PocketError::Storage {
                message: e.to_string(),
            })?;
        }
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_and_verifies() {
        let store = Store::open_in_memory().unwrap();
        let log = AuditLog::new(&store);
        log.audit("owner", "server.on", "mac-host", "{}").unwrap();
        log.audit("owner", "model.loaded", "mlx-community/x@rev", "{}")
            .unwrap();
        assert_eq!(log.verify().unwrap(), None);
    }

    #[test]
    fn export_jsonl_writes_one_line_per_entry() {
        let store = Store::open_in_memory().unwrap();
        let log = AuditLog::new(&store);
        log.audit("owner", "offline_mode.toggled", "settings", "{\"on\":true}")
            .unwrap();
        log.audit(
            "owner",
            "license.activated",
            "pro",
            "{\"serial\":\"01J...\"}",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        log.export_jsonl(path.to_str().unwrap()).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed["hash"].is_string());
        }
    }

    #[test]
    fn detects_tampering_through_the_service() {
        let store = Store::open_in_memory().unwrap();
        let log = AuditLog::new(&store);
        log.audit("owner", "device.paired", "dev1", "{}").unwrap();
        log.audit("owner", "device.revoked", "dev1", "{}").unwrap();
        store
            .writer()
            .execute("UPDATE audit SET actor = 'attacker' WHERE id = 1", [])
            .unwrap();
        assert_eq!(log.verify().unwrap(), Some(1));
    }
}
