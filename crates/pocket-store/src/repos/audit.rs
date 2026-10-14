use rusqlite::params;
use sha2::{Digest, Sha256};

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub id: i64,
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail_json: String,
    pub prev_hash: Vec<u8>,
    pub hash: Vec<u8>,
}

pub struct AuditRepo<'a> {
    store: &'a Store,
}

impl<'a> AuditRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    fn last_hash(&self) -> rusqlite::Result<Vec<u8>> {
        self.store
            .reader()
            .query_row(
                "SELECT hash FROM audit ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .or_else(|e| {
                if e == rusqlite::Error::QueryReturnedNoRows {
                    Ok(Vec::new())
                } else {
                    Err(e)
                }
            })
    }

    /// Appends a tamper-evident row: `hash = sha256(prev_hash || ts || actor || action || target || detail_json)`.
    pub fn append(
        &self,
        ts: i64,
        actor: &str,
        action: &str,
        target: &str,
        detail_json: &str,
    ) -> rusqlite::Result<i64> {
        let prev_hash = self.last_hash()?;
        let mut hasher = Sha256::new();
        hasher.update(&prev_hash);
        hasher.update(ts.to_le_bytes());
        hasher.update(actor.as_bytes());
        hasher.update(action.as_bytes());
        hasher.update(target.as_bytes());
        hasher.update(detail_json.as_bytes());
        let hash = hasher.finalize().to_vec();

        let conn = self.store.writer();
        conn.execute(
            "INSERT INTO audit (ts, actor, action, target, detail_json, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![ts, actor, action, target, detail_json, prev_hash, hash],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list(&self, limit: u32) -> rusqlite::Result<Vec<AuditRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, action, target, detail_json, prev_hash, hash FROM audit ORDER BY id ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(AuditRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                actor: row.get(2)?,
                action: row.get(3)?,
                target: row.get(4)?,
                detail_json: row.get(5)?,
                prev_hash: row.get(6)?,
                hash: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Recomputes each row's hash from its predecessor and compares; returns
    /// the id of the first row whose chain breaks, or `None` if intact.
    pub fn verify_chain(&self) -> rusqlite::Result<Option<i64>> {
        let rows = self.list(u32::MAX)?;
        let mut prev = Vec::new();
        for row in rows {
            let mut hasher = Sha256::new();
            hasher.update(&prev);
            hasher.update(row.ts.to_le_bytes());
            hasher.update(row.actor.as_bytes());
            hasher.update(row.action.as_bytes());
            hasher.update(row.target.as_bytes());
            hasher.update(row.detail_json.as_bytes());
            let expected = hasher.finalize().to_vec();
            if expected != row.hash || prev != row.prev_hash {
                return Ok(Some(row.id));
            }
            prev = row.hash;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_form_a_hash_chain() {
        let store = Store::open_in_memory().unwrap();
        let repo = AuditRepo::new(&store);
        repo.append(1, "owner", "device.paired", "dev1", "{}")
            .unwrap();
        repo.append(2, "owner", "device.revoked", "dev1", "{}")
            .unwrap();
        let rows = repo.list(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].prev_hash, Vec::<u8>::new());
        assert_eq!(rows[1].prev_hash, rows[0].hash);
        assert_eq!(repo.verify_chain().unwrap(), None);
    }

    #[test]
    fn detects_tampering() {
        let store = Store::open_in_memory().unwrap();
        let repo = AuditRepo::new(&store);
        repo.append(1, "owner", "device.paired", "dev1", "{}")
            .unwrap();
        repo.append(2, "owner", "device.revoked", "dev1", "{}")
            .unwrap();
        store
            .writer()
            .execute("UPDATE audit SET target = 'tampered' WHERE id = 1", [])
            .unwrap();
        assert_eq!(repo.verify_chain().unwrap(), Some(1));
    }
}
