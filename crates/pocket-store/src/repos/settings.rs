use crate::Store;

pub struct SettingsRepo<'a> {
    store: &'a Store,
}

impl<'a> SettingsRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.store
            .reader()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> rusqlite::Result<()> {
        self.store
            .writer()
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let repo = SettingsRepo::new(&store);
        assert_eq!(repo.get("offline_mode").unwrap(), None);
        repo.set("offline_mode", "true").unwrap();
        assert_eq!(repo.get("offline_mode").unwrap(), Some("true".to_string()));
        repo.set("offline_mode", "false").unwrap();
        assert_eq!(repo.get("offline_mode").unwrap(), Some("false".to_string()));
        repo.delete("offline_mode").unwrap();
        assert_eq!(repo.get("offline_mode").unwrap(), None);
    }
}
