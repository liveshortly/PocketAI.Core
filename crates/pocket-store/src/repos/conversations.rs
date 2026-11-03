use serde_json::json;

use crate::{sync, Store};

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRow {
    pub id: String,
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub model_id: Option<String>,
    pub system_prompt: Option<String>,
    pub params_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived: bool,
}

pub struct ConversationsRepo<'a> {
    store: &'a Store,
}

impl<'a> ConversationsRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    // NET-22: every write is a sync op; `sync::emit` records it and applies it
    // to this table in one transaction.
    pub fn create(&self, row: &ConversationRow) -> rusqlite::Result<()> {
        let value = sync::conversation_value(row);
        sync::emit(self.store, "conv", &row.id, &row.id, "create", value, row.created_at)
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<ConversationRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, title, project_id, model_id, system_prompt, params_json, created_at, updated_at, archived
                 FROM conversations WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn update_title(&self, id: &str, title: &str, updated_at: i64) -> rusqlite::Result<()> {
        sync::emit(self.store, "conv", id, id, "title", json!(title), updated_at)
    }

    pub fn update_model(&self, id: &str, model_id: &str, updated_at: i64) -> rusqlite::Result<()> {
        sync::emit(self.store, "conv", id, id, "model_id", json!(model_id), updated_at)
    }

    pub fn archive(&self, id: &str, archived: bool) -> rusqlite::Result<()> {
        sync::emit(self.store, "conv", id, id, "archived", json!(archived), sync::now_ms())
    }

    /// Tombstone: removes the chat here and, once synced, on the peer.
    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        sync::emit(self.store, "conv", id, id, "delete", serde_json::Value::Null, sync::now_ms())
    }

    pub fn list(&self) -> rusqlite::Result<Vec<ConversationRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, title, project_id, model_id, system_prompt, params_json, created_at, updated_at, archived
             FROM conversations ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<ConversationRow> {
        Ok(ConversationRow {
            id: row.get(0)?,
            title: row.get(1)?,
            project_id: row.get(2)?,
            model_id: row.get(3)?,
            system_prompt: row.get(4)?,
            params_json: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            archived: row.get::<_, i64>(8)? != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> ConversationRow {
        ConversationRow {
            id: id.to_string(),
            title: Some("New chat".into()),
            project_id: None,
            model_id: None,
            system_prompt: None,
            params_json: None,
            created_at: 1,
            updated_at: 1,
            archived: false,
        }
    }

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let repo = ConversationsRepo::new(&store);
        repo.create(&sample("c1")).unwrap();
        assert_eq!(
            repo.get("c1").unwrap().unwrap().title,
            Some("New chat".into())
        );

        repo.update_title("c1", "Renamed", 2).unwrap();
        assert_eq!(
            repo.get("c1").unwrap().unwrap().title,
            Some("Renamed".into())
        );

        repo.archive("c1", true).unwrap();
        assert!(repo.get("c1").unwrap().unwrap().archived);

        assert_eq!(repo.list().unwrap().len(), 1);
        repo.delete("c1").unwrap();
        assert_eq!(repo.get("c1").unwrap(), None);
    }
}
