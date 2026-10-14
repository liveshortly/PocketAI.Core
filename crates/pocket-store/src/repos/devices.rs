use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub public_key: Vec<u8>,
    pub token_hash: Vec<u8>,
    pub role: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
    pub last_addr: Option<String>,
    pub revoked_at: Option<i64>,
}

pub struct DevicesRepo<'a> {
    store: &'a Store,
}

impl<'a> DevicesRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn create(&self, row: &DeviceRow) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO devices (id, name, platform, public_key, token_hash, role, paired_at, last_seen_at, last_addr, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![row.id, row.name, row.platform, row.public_key, row.token_hash, row.role,
                row.paired_at, row.last_seen_at, row.last_addr, row.revoked_at],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<DeviceRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, name, platform, public_key, token_hash, role, paired_at, last_seen_at, last_addr, revoked_at
                 FROM devices WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn revoke(&self, id: &str, at: i64) -> rusqlite::Result<()> {
        self.store
            .writer()
            .execute("UPDATE devices SET revoked_at = ?2 WHERE id = ?1", params![id, at])?;
        Ok(())
    }

    pub fn mark_seen(&self, id: &str, at: i64, addr: &str) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "UPDATE devices SET last_seen_at = ?2, last_addr = ?3 WHERE id = ?1",
            params![id, at, addr],
        )?;
        Ok(())
    }

    pub fn list(&self) -> rusqlite::Result<Vec<DeviceRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, name, platform, public_key, token_hash, role, paired_at, last_seen_at, last_addr, revoked_at FROM devices",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store.writer().execute("DELETE FROM devices WHERE id = ?1", [id])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<DeviceRow> {
        Ok(DeviceRow {
            id: row.get(0)?,
            name: row.get(1)?,
            platform: row.get(2)?,
            public_key: row.get(3)?,
            token_hash: row.get(4)?,
            role: row.get(5)?,
            paired_at: row.get(6)?,
            last_seen_at: row.get(7)?,
            last_addr: row.get(8)?,
            revoked_at: row.get(9)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> DeviceRow {
        DeviceRow {
            id: id.to_string(),
            name: "Rohit's iPhone".into(),
            platform: "ios".into(),
            public_key: vec![1, 2, 3],
            token_hash: vec![4, 5, 6],
            role: "owner".into(),
            paired_at: 1,
            last_seen_at: None,
            last_addr: None,
            revoked_at: None,
        }
    }

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let repo = DevicesRepo::new(&store);
        repo.create(&sample("dev1")).unwrap();
        assert!(repo.get("dev1").unwrap().unwrap().revoked_at.is_none());

        repo.mark_seen("dev1", 42, "192.168.1.5").unwrap();
        let seen = repo.get("dev1").unwrap().unwrap();
        assert_eq!(seen.last_seen_at, Some(42));
        assert_eq!(seen.last_addr, Some("192.168.1.5".into()));

        repo.revoke("dev1", 99).unwrap();
        assert_eq!(repo.get("dev1").unwrap().unwrap().revoked_at, Some(99));

        assert_eq!(repo.list().unwrap().len(), 1);
        repo.delete("dev1").unwrap();
        assert_eq!(repo.get("dev1").unwrap(), None);
    }
}
