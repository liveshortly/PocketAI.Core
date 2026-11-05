use rusqlite::params;

use crate::{sync, Store};

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

    // NET-22: every write is a sync op (see `sync::emit_message`).
    pub fn create(&self, row: &MessageRow) -> rusqlite::Result<()> {
        sync::emit_message(self.store, row, None)
    }

    /// A regenerated reply: kept beside `sibling_of` (never overwrites it);
    /// the newest sibling is the one `list_for_conversation` shows.
    pub fn create_sibling(&self, row: &MessageRow, sibling_of: &str) -> rusqlite::Result<()> {
        sync::emit_message(self.store, row, Some(sibling_of))
    }

    /// Not a sync op: nothing streams partial replies into the store today,
    /// and only final messages sync (append-only).
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
             FROM messages m WHERE conversation_id = ?1 AND NOT EXISTS (
               SELECT 1 FROM messages s WHERE s.conversation_id = m.conversation_id AND s.id != m.id
                 AND COALESCE(s.sibling_of, s.id) = COALESCE(m.sibling_of, m.id)
                 AND (s.created_at, s.lamport, s.id) > (m.created_at, m.lamport, m.id))
             ORDER BY created_at, lamport, id",
        )?;
        let rows = stmt.query_map([conversation_id], Self::map_row)?;
        rows.collect()
    }

    /// Tombstone (synced).
    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        let Some(row) = self.get(id)? else { return Ok(()) };
        sync::emit(self.store, "msg", id, &row.conversation_id, "delete", serde_json::Value::Null, sync::now_ms())
    }

    /// UI-68 (edit in place): removes `id` and every message at or after it in
    /// the conversation, hidden regenerate-siblings included, as synced
    /// deletes. Returns how many rows went; 0 when `id` is unknown.
    pub fn delete_from(&self, id: &str) -> rusqlite::Result<usize> {
        let Some(target) = self.get(id)? else { return Ok(0) };
        let ids: Vec<String> = {
            let conn = self.store.reader();
            let mut stmt = conn.prepare(
                "SELECT id FROM messages WHERE conversation_id = ?1 AND created_at >= ?2",
            )?;
            let rows = stmt.query_map(params![target.conversation_id, target.created_at], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for m in &ids {
            self.delete(m)?;
        }
        Ok(ids.len())
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
    fn delete_from_removes_target_and_everything_after() {
        let store = Store::open_in_memory().unwrap();
        ConversationsRepo::new(&store).create(&conversation("c1")).unwrap();
        let repo = MessagesRepo::new(&store);
        for (id, at) in [("m1", 1), ("m2", 2), ("m3", 3), ("m4", 4)] {
            let mut row = message(id, "c1");
            row.created_at = at;
            repo.create(&row).unwrap();
        }
        assert_eq!(repo.delete_from("m3").unwrap(), 2);
        let left: Vec<String> = repo.list_for_conversation("c1").unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(left, ["m1", "m2"]);
        assert_eq!(repo.delete_from("nope").unwrap(), 0);
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
