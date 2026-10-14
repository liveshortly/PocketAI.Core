use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use rusqlite::Connection;

const MIGRATIONS: &[(&i64, &str)] = &[(&1, include_str!("../migrations/0001_init.sql"))];
const READER_POOL_SIZE: usize = 4;

pub struct Store {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(())
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (target, sql) in MIGRATIONS {
        if version < **target {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", **target)?;
        }
    }
    Ok(())
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let path = path.as_ref();
        let writer = Connection::open(path)?;
        configure(&writer)?;
        migrate(&writer)?;

        let mut readers = Vec::with_capacity(READER_POOL_SIZE);
        for _ in 0..READER_POOL_SIZE {
            let conn = Connection::open(path)?;
            configure(&conn)?;
            readers.push(Mutex::new(conn));
        }

        Ok(Self {
            writer: Mutex::new(writer),
            readers,
            next_reader: AtomicUsize::new(0),
        })
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let writer = Connection::open_in_memory()?;
        // In-memory DBs cannot share state across connections unless using a
        // shared cache URI; tests that need the reader pool should open a
        // temp file instead. Mirror the writer into the reader slots so
        // single-connection call sites still work.
        configure(&writer)?;
        migrate(&writer)?;
        Ok(Self {
            writer: Mutex::new(writer),
            readers: Vec::new(),
            next_reader: AtomicUsize::new(0),
        })
    }

    pub fn writer(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.writer.lock().unwrap()
    }

    pub fn reader(&self) -> std::sync::MutexGuard<'_, Connection> {
        if self.readers.is_empty() {
            return self.writer();
        }
        let idx = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        self.readers[idx].lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_runs_migration_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pocket.sqlite");
        {
            let store = Store::open(&path).unwrap();
            let version: i64 = store
                .writer()
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, 1);
        }
        // Re-opening must be idempotent (no error re-running migrations).
        let store = Store::open(&path).unwrap();
        let version: i64 = store
            .writer()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pocket.sqlite")).unwrap();
        let mode: String = store
            .writer()
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn concurrent_read_during_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("pocket.sqlite")).unwrap();
        let writer = store.writer();
        // A reader connection (WAL mode) can read while the writer holds its lock.
        let reader = store.reader();
        let count: i64 = reader
            .query_row("SELECT count(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        drop(writer);
    }
}
