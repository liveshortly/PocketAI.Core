use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content_json: String,
    pub reasoning: Option<String>,
    pub stats_json: Option<String>,
    pub model_id: Option<String>,
    pub created_at: i64,
    pub status: String,
}

pub struct MessagesRepo<'a> {
    store: &'a Store,
}

impl<'a> MessagesRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn create(&self, row: &MessageRow) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO messages (id, conversation_id, role, content_json, reasoning, stats_json, model_id, created_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![row.id, row.conversation_id, row.role, row.content_json, row.reasoning,
                row.stats_json, row.model_id, row.created_at, row.status],
        )?;
        Ok(())
    }

    pub fn upsert_partial(
        &self,
        id: &str,
        content_json: &str,
        status: &str,
    ) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "UPDATE messages SET content_json = ?2, status = ?3 WHERE id = ?1",
            params![id, content_json, status],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<MessageRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, conversation_id, role, content_json, reasoning, stats_json, model_id, created_at, status
                 FROM messages WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn list_for_conversation(
        &self,
        conversation_id: &str,
    ) -> rusqlite::Result<Vec<MessageRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, role, content_json, reasoning, stats_json, model_id, created_at, status
             FROM messages WHERE conversation_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([conversation_id], Self::map_row)?;
        rows.collect()
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store
            .writer()
            .execute("DELETE FROM messages WHERE id = ?1", [id])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
        Ok(MessageRow {
            id: row.get(0)?,
            conversation_id: row.get(1)?,
            role: row.get(2)?,
            content_json: row.get(3)?,
            reasoning: row.get(4)?,
            stats_json: row.get(5)?,
            model_id: row.get(6)?,
            created_at: row.get(7)?,
            status: row.get(8)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos::conversations::{ConversationRow, ConversationsRepo};

    fn conversation(id: &str) -> ConversationRow {
        ConversationRow {
            id: id.to_string(),
            title: None,
            project_id: None,
            model_id: None,
            system_prompt: None,
            params_json: None,
            created_at: 1,
            updated_at: 1,
            archived: false,
        }
    }

    fn message(id: &str, conversation_id: &str) -> MessageRow {
        MessageRow {
            id: id.to_string(),
            conversation_id: conversation_id.to_string(),
            role: "user".into(),
            content_json: "{}".into(),
            reasoning: None,
            stats_json: None,
            model_id: None,
            created_at: 1,
            status: "done".into(),
        }
    }

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        ConversationsRepo::new(&store)
            .create(&conversation("c1"))
            .unwrap();
        let repo = MessagesRepo::new(&store);
        repo.create(&message("m1", "c1")).unwrap();

        repo.upsert_partial("m1", "{\"text\":\"hi\"}", "streaming")
            .unwrap();
        let got = repo.get("m1").unwrap().unwrap();
        assert_eq!(got.status, "streaming");

        assert_eq!(repo.list_for_conversation("c1").unwrap().len(), 1);
        repo.delete("m1").unwrap();
        assert_eq!(repo.get("m1").unwrap(), None);
    }

    #[test]
    fn cascade_delete_removes_messages_when_conversation_deleted() {
        let store = Store::open_in_memory().unwrap();
        let conversations = ConversationsRepo::new(&store);
        conversations.create(&conversation("c1")).unwrap();
        let messages = MessagesRepo::new(&store);
        messages.create(&message("m1", "c1")).unwrap();
        messages.create(&message("m2", "c1")).unwrap();

        conversations.delete("c1").unwrap();

        assert_eq!(messages.list_for_conversation("c1").unwrap().len(), 0);
        assert_eq!(messages.get("m1").unwrap(), None);
        assert_eq!(messages.get("m2").unwrap(), None);
    }
}
