use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRow {
    pub id: String,
    pub manifest_json: String,
    pub state: String,
    pub bytes_done: u64,
    pub installed_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub pinned_engine: Option<String>,
    pub benchmark_json: Option<String>,
}

pub struct ModelsRepo<'a> {
    store: &'a Store,
}

impl<'a> ModelsRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn upsert(&self, row: &ModelRow) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO models (id, manifest_json, state, bytes_done, installed_at, last_used_at, pinned_engine, benchmark_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET manifest_json=excluded.manifest_json, state=excluded.state,
               bytes_done=excluded.bytes_done, installed_at=excluded.installed_at,
               last_used_at=excluded.last_used_at, pinned_engine=excluded.pinned_engine,
               benchmark_json=excluded.benchmark_json",
            params![
                row.id, row.manifest_json, row.state, row.bytes_done as i64,
                row.installed_at, row.last_used_at, row.pinned_engine, row.benchmark_json
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<ModelRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, manifest_json, state, bytes_done, installed_at, last_used_at, pinned_engine, benchmark_json
                 FROM models WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store.writer().execute("DELETE FROM models WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn list(&self) -> rusqlite::Result<Vec<ModelRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, manifest_json, state, bytes_done, installed_at, last_used_at, pinned_engine, benchmark_json FROM models",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<ModelRow> {
        Ok(ModelRow {
            id: row.get(0)?,
            manifest_json: row.get(1)?,
            state: row.get(2)?,
            bytes_done: row.get::<_, i64>(3)? as u64,
            installed_at: row.get(4)?,
            last_used_at: row.get(5)?,
            pinned_engine: row.get(6)?,
            benchmark_json: row.get(7)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> ModelRow {
        ModelRow {
            id: id.to_string(),
            manifest_json: "{}".into(),
            state: "available".into(),
            bytes_done: 0,
            installed_at: None,
            last_used_at: None,
            pinned_engine: None,
            benchmark_json: None,
        }
    }

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let repo = ModelsRepo::new(&store);
        assert_eq!(repo.get("m1").unwrap(), None);
        repo.upsert(&sample("m1")).unwrap();
        let got = repo.get("m1").unwrap().unwrap();
        assert_eq!(got.state, "available");

        let mut updated = got.clone();
        updated.state = "installed".into();
        updated.bytes_done = 1024;
        repo.upsert(&updated).unwrap();
        assert_eq!(repo.get("m1").unwrap().unwrap().state, "installed");

        assert_eq!(repo.list().unwrap().len(), 1);
        repo.delete("m1").unwrap();
        assert_eq!(repo.get("m1").unwrap(), None);
    }
}
