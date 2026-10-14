use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct EgressRow {
    pub id: i64,
    pub ts: i64,
    pub host: String,
    pub purpose: String,
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub initiated_by: String,
}

pub struct EgressRepo<'a> {
    store: &'a Store,
}

impl<'a> EgressRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn record(
        &self,
        ts: i64,
        host: &str,
        purpose: &str,
        bytes_out: u64,
        bytes_in: u64,
        initiated_by: &str,
    ) -> rusqlite::Result<i64> {
        let conn = self.store.writer();
        conn.execute(
            "INSERT INTO egress (ts, host, purpose, bytes_out, bytes_in, initiated_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![ts, host, purpose, bytes_out as i64, bytes_in as i64, initiated_by],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list(&self, limit: u32) -> rusqlite::Result<Vec<EgressRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, ts, host, purpose, bytes_out, bytes_in, initiated_by FROM egress ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(EgressRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                host: row.get(2)?,
                purpose: row.get(3)?,
                bytes_out: row.get::<_, i64>(4)? as u64,
                bytes_in: row.get::<_, i64>(5)? as u64,
                initiated_by: row.get(6)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_list() {
        let store = Store::open_in_memory().unwrap();
        let repo = EgressRepo::new(&store);
        repo.record(1, "huggingface.co", "catalog_refresh", 100, 2000, "app")
            .unwrap();
        repo.record(
            2,
            "huggingface.co",
            "model_download",
            0,
            4_000_000_000,
            "app",
        )
        .unwrap();
        let rows = repo.list(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].purpose, "model_download");
    }
}
