use rusqlite::params;

use crate::Store;

/// One row of the agent journal (AGT-02): every approval transition.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentEventRow {
    pub id: i64,
    pub ts: i64,
    pub session_id: String,
    pub approval_id: Option<String>,
    pub kind: String,
    pub detail_json: String,
}

pub struct AgentEventsRepo<'a> {
    store: &'a Store,
}

impl<'a> AgentEventsRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn append(
        &self,
        ts: i64,
        session_id: &str,
        approval_id: Option<&str>,
        kind: &str,
        detail_json: &str,
    ) -> rusqlite::Result<i64> {
        let conn = self.store.writer();
        conn.execute(
            "INSERT INTO agent_events (ts, session_id, approval_id, kind, detail_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, session_id, approval_id, kind, detail_json],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Oldest first.
    pub fn list(&self, limit: u32) -> rusqlite::Result<Vec<AgentEventRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, ts, session_id, approval_id, kind, detail_json FROM agent_events ORDER BY id ASC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(AgentEventRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                session_id: row.get(2)?,
                approval_id: row.get(3)?,
                kind: row.get(4)?,
                detail_json: row.get(5)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_then_list_in_order() {
        let store = Store::open_in_memory().unwrap();
        let repo = AgentEventsRepo::new(&store);
        repo.append(1, "s1", Some("a1"), "approval.approved", "{}").unwrap();
        repo.append(2, "s1", None, "kill_switch", "{}").unwrap();
        let rows = repo.list(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].approval_id.as_deref(), Some("a1"));
        assert_eq!(rows[1].kind, "kill_switch");
        assert_eq!(rows[1].approval_id, None);
    }
}
