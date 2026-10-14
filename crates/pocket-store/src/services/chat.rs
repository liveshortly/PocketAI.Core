use crate::repos::conversations::{ConversationRow, ConversationsRepo};
use crate::repos::messages::{MessageRow, MessagesRepo};
use crate::Store;

/// CORE-11: the conversation/message domain service other crates reach for,
/// built over the plain repos from CORE-03 - search/pagination over
/// conversations, and Markdown export of a conversation's messages.
pub struct ChatService<'a> {
    conversations: ConversationsRepo<'a>,
    messages: MessagesRepo<'a>,
}

#[derive(Debug, Clone, Default)]
pub struct ConversationQuery {
    pub search: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl<'a> ChatService<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self {
            conversations: ConversationsRepo::new(store),
            messages: MessagesRepo::new(store),
        }
    }

    pub fn create_conversation(&self, row: &ConversationRow) -> rusqlite::Result<()> {
        self.conversations.create(row)
    }

    /// Title search (case-insensitive substring), paginated. Default page
    /// size 50 when `limit` is 0.
    pub fn list_conversations(&self, query: &ConversationQuery) -> rusqlite::Result<Vec<ConversationRow>> {
        let mut all = self.conversations.list()?;
        if let Some(search) = &query.search {
            let needle = search.to_lowercase();
            all.retain(|c| {
                c.title
                    .as_deref()
                    .map(|t| t.to_lowercase().contains(&needle))
                    .unwrap_or(false)
            });
        }
        let limit = if query.limit == 0 { 50 } else { query.limit };
        Ok(all.into_iter().skip(query.offset).take(limit).collect())
    }

    pub fn create_message(&self, row: &MessageRow) -> rusqlite::Result<()> {
        self.messages.create(row)
    }

    /// Partial-message upsert used while a reply streams in.
    pub fn upsert_partial_message(&self, id: &str, content_json: &str, status: &str) -> rusqlite::Result<()> {
        self.messages.upsert_partial(id, content_json, status)
    }

    pub fn messages_for(&self, conversation_id: &str) -> rusqlite::Result<Vec<MessageRow>> {
        self.messages.list_for_conversation(conversation_id)
    }

    pub fn archive(&self, id: &str, archived: bool) -> rusqlite::Result<()> {
        self.conversations.archive(id, archived)
    }

    /// `# Title`, then `**You:**`/`**Model (<name>):**` turns, per CORE-11.
    /// Message `content_json` is expected to be `{"text": "..."}`-shaped;
    /// anything else renders as an empty body rather than erroring, since
    /// export is a best-effort convenience, not a contract surface.
    pub fn export_markdown(&self, conversation_id: &str) -> rusqlite::Result<String> {
        let title = self
            .conversations
            .get(conversation_id)?
            .and_then(|c| c.title)
            .unwrap_or_else(|| "Untitled".to_string());
        let messages = self.messages_for(conversation_id)?;

        let mut out = format!("# {title}\n\n");
        for message in messages {
            let label = match message.role.as_str() {
                "user" => "**You:**".to_string(),
                "assistant" => {
                    let model = message.model_id.as_deref().unwrap_or("Model");
                    format!("**Model ({model}):**")
                }
                other => format!("**{other}:**"),
            };
            let text = extract_text(&message.content_json);
            out.push_str(&label);
            out.push(' ');
            out.push_str(&text);
            out.push_str("\n\n");
        }
        Ok(out)
    }
}

fn extract_text(content_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(content_json)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(id: &str, title: &str) -> ConversationRow {
        ConversationRow {
            id: id.to_string(),
            title: Some(title.to_string()),
            project_id: None,
            model_id: None,
            system_prompt: None,
            params_json: None,
            created_at: 1,
            updated_at: 1,
            archived: false,
        }
    }

    fn message(id: &str, conversation_id: &str, role: &str, text: &str, model_id: Option<&str>) -> MessageRow {
        MessageRow {
            id: id.to_string(),
            conversation_id: conversation_id.to_string(),
            role: role.to_string(),
            content_json: serde_json::json!({ "text": text }).to_string(),
            reasoning: None,
            stats_json: None,
            model_id: model_id.map(str::to_string),
            created_at: 1,
            status: "done".to_string(),
        }
    }

    #[test]
    fn searches_conversations_by_title_case_insensitively() {
        let store = Store::open_in_memory().unwrap();
        let service = ChatService::new(&store);
        service.create_conversation(&conversation("c1", "Rust borrow checker")).unwrap();
        service.create_conversation(&conversation("c2", "Swift concurrency")).unwrap();

        let results = service
            .list_conversations(&ConversationQuery {
                search: Some("rust".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "c1");
    }

    #[test]
    fn paginates_conversations_with_default_page_size_50() {
        let store = Store::open_in_memory().unwrap();
        let service = ChatService::new(&store);
        for i in 0..5 {
            service
                .create_conversation(&conversation(&format!("c{i}"), &format!("Chat {i}")))
                .unwrap();
        }
        let page = service
            .list_conversations(&ConversationQuery {
                limit: 2,
                offset: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn export_markdown_snapshot() {
        let store = Store::open_in_memory().unwrap();
        let service = ChatService::new(&store);
        service.create_conversation(&conversation("c1", "Rust borrow checker")).unwrap();
        service
            .create_message(&message("m1", "c1", "user", "Why does this not compile?", None))
            .unwrap();
        service
            .create_message(&message(
                "m2",
                "c1",
                "assistant",
                "Because the borrow outlives the value.",
                Some("Qwen3 4B"),
            ))
            .unwrap();

        let markdown = service.export_markdown("c1").unwrap();
        assert_eq!(
            markdown,
            "# Rust borrow checker\n\n\
             **You:** Why does this not compile?\n\n\
             **Model (Qwen3 4B):** Because the borrow outlives the value.\n\n"
        );
    }
}
