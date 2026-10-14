use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct PeerRow {
    pub id: String,
    pub name: String,
    pub cert_sha256: Vec<u8>,
    pub addrs_json: String,
    pub port: u16,
    pub token_ref: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
}

pub struct PeersRepo<'a> {
    store: &'a Store,
}

impl<'a> PeersRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn create(&self, row: &PeerRow) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO peers (id, name, cert_sha256, addrs_json, port, token_ref, paired_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![row.id, row.name, row.cert_sha256, row.addrs_json, row.port, row.token_ref,
                row.paired_at, row.last_seen_at],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<PeerRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, name, cert_sha256, addrs_json, port, token_ref, paired_at, last_seen_at FROM peers WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn mark_seen(&self, id: &str, at: i64) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "UPDATE peers SET last_seen_at = ?2 WHERE id = ?1",
            params![id, at],
        )?;
        Ok(())
    }

    pub fn list(&self) -> rusqlite::Result<Vec<PeerRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, name, cert_sha256, addrs_json, port, token_ref, paired_at, last_seen_at FROM peers",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store
            .writer()
            .execute("DELETE FROM peers WHERE id = ?1", [id])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<PeerRow> {
        Ok(PeerRow {
            id: row.get(0)?,
            name: row.get(1)?,
            cert_sha256: row.get(2)?,
            addrs_json: row.get(3)?,
            port: row.get::<_, i64>(4)? as u16,
            token_ref: row.get(5)?,
            paired_at: row.get(6)?,
            last_seen_at: row.get(7)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let repo = PeersRepo::new(&store);
        repo.create(&PeerRow {
            id: "peer1".into(),
            name: "Rohit's Mac".into(),
            cert_sha256: vec![9, 9],
            addrs_json: "[\"192.168.1.2\"]".into(),
            port: 7480,
            token_ref: "keychain:peer1".into(),
            paired_at: 1,
            last_seen_at: None,
        })
        .unwrap();
        assert_eq!(repo.get("peer1").unwrap().unwrap().port, 7480);
        repo.mark_seen("peer1", 10).unwrap();
        assert_eq!(repo.get("peer1").unwrap().unwrap().last_seen_at, Some(10));
        assert_eq!(repo.list().unwrap().len(), 1);
        repo.delete("peer1").unwrap();
        assert_eq!(repo.get("peer1").unwrap(), None);
    }
}
